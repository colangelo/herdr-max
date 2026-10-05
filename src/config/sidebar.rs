use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::detect::Agent;

const MAX_SIDEBAR_ROWS: usize = 16;
const MAX_SIDEBAR_TOKENS_PER_ROW: usize = 16;
const DEFAULT_SIDEBAR_ROW_GAP: u16 = 0;

fn deserialize_sidebar_rows<'de, D, T>(deserializer: D) -> Result<Vec<Vec<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let rows = Vec::<Vec<T>>::deserialize(deserializer)?;
    validate_sidebar_rows(&rows).map_err(serde::de::Error::custom)?;
    Ok(rows)
}

fn validate_sidebar_rows<T>(rows: &[Vec<T>]) -> Result<(), String> {
    if rows.len() > MAX_SIDEBAR_ROWS {
        return Err(format!(
            "sidebar layouts may contain at most {MAX_SIDEBAR_ROWS} rows"
        ));
    }
    if rows
        .iter()
        .any(|row| row.len() > MAX_SIDEBAR_TOKENS_PER_ROW)
    {
        return Err(format!(
            "sidebar rows may contain at most {MAX_SIDEBAR_TOKENS_PER_ROW} tokens"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarTokenColor {
    r: u8,
    g: u8,
    b: u8,
}

impl SidebarTokenColor {
    pub(crate) fn ratatui(self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(self.r, self.g, self.b)
    }
}

impl Serialize for SidebarTokenColor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b))
    }
}

impl<'de> Deserialize<'de> for SidebarTokenColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let hex = value.strip_prefix('#').filter(|hex| {
            hex.is_ascii()
                && matches!(hex.len(), 3 | 6)
                && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        let Some(hex) = hex else {
            return Err(serde::de::Error::custom(
                "sidebar token fg must be #RGB or #RRGGBB",
            ));
        };
        let (r, g, b) = if hex.len() == 3 {
            let mut digits = hex
                .bytes()
                .map(|byte| char::from(byte).to_digit(16).expect("validated hex digit") as u8 * 17);
            (
                digits.next().expect("three hex digits"),
                digits.next().expect("three hex digits"),
                digits.next().expect("three hex digits"),
            )
        } else {
            (
                u8::from_str_radix(&hex[0..2], 16).expect("validated hex digits"),
                u8::from_str_radix(&hex[2..4], 16).expect("validated hex digits"),
                u8::from_str_radix(&hex[4..6], 16).expect("validated hex digits"),
            )
        };
        Ok(Self { r, g, b })
    }
}

/// Which end of a token loses cells when the row is too narrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarTokenTruncate {
    /// Cut the end, keep the beginning, `text…` (the default).
    End,
    /// Cut the beginning, keep the end, `…text`.
    Start,
    /// A value this build does not know: treated as `End`, reported as a
    /// diagnostic (fork issue 164).
    Unrecognized,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SidebarTokenStyle {
    pub fg: Option<SidebarTokenColor>,
    pub bold: Option<bool>,
    pub dim: Option<bool>,
    pub italic: Option<bool>,
    /// The token gets its full width before other flexible tokens in the row
    /// shrink, and is dropped last.
    pub keep: Option<bool>,
    pub truncate: Option<SidebarTokenTruncate>,
    /// Style keys whose value had the wrong type, as `STYLE_KEY_*` bits: the
    /// value is ignored and `sidebar_style_diagnostics` names it by position
    /// (fork issue 163).
    pub invalid: u8,
}

const STYLE_KEY_FG: u8 = 1;
const STYLE_KEY_BOLD: u8 = 2;
const STYLE_KEY_DIM: u8 = 4;
const STYLE_KEY_ITALIC: u8 = 8;
const STYLE_KEY_KEEP: u8 = 16;
const STYLE_KEY_TRUNCATE: u8 = 32;

impl SidebarTokenStyle {
    /// The style keys whose value was ignored for having the wrong type, with
    /// what each expects.
    pub(crate) fn invalid_keys(&self) -> Vec<(&'static str, &'static str)> {
        [
            (STYLE_KEY_FG, "fg", "a #RGB or #RRGGBB colour"),
            (STYLE_KEY_BOLD, "bold", "true or false"),
            (STYLE_KEY_DIM, "dim", "true or false"),
            (STYLE_KEY_ITALIC, "italic", "true or false"),
            (STYLE_KEY_KEEP, "keep", "true or false"),
            (STYLE_KEY_TRUNCATE, "truncate", "\"start\" or \"end\""),
        ]
        .into_iter()
        .filter(|(bit, ..)| self.invalid & bit != 0)
        .map(|(_, key, expected)| (key, expected))
        .collect()
    }

    pub(crate) fn keeps_width(&self) -> bool {
        self.keep == Some(true)
    }

    pub(crate) fn truncates_start(&self) -> bool {
        self.truncate == Some(SidebarTokenTruncate::Start)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSidebarToken {
    StateIcon,
    StateText,
    Workspace,
    Tab,
    Pane,
    Agent,
    TerminalTitle,
    TerminalTitleStripped,
    Custom(String),
    Styled {
        token: Box<AgentSidebarToken>,
        style: SidebarTokenStyle,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceSidebarToken {
    StateIcon,
    StateText,
    Workspace,
    Branch,
    GitStatus,
    Custom(String),
    Styled {
        token: Box<SpaceSidebarToken>,
        style: SidebarTokenStyle,
    },
}

impl AgentSidebarToken {
    pub(crate) fn parts(&self) -> (&Self, SidebarTokenStyle) {
        match self {
            Self::Styled { token, style } => (token, *style),
            token => (token, SidebarTokenStyle::default()),
        }
    }
}

impl SpaceSidebarToken {
    pub(crate) fn parts(&self) -> (&Self, SidebarTokenStyle) {
        match self {
            Self::Styled { token, style } => (token, *style),
            token => (token, SidebarTokenStyle::default()),
        }
    }
}

struct RawStyledSidebarToken {
    token: String,
    fg: Option<SidebarTokenColor>,
    bold: Option<bool>,
    dim: Option<bool>,
    italic: Option<bool>,
    keep: Option<bool>,
    truncate: Option<SidebarTokenTruncate>,
    invalid: u8,
}

enum RawSidebarToken {
    Plain(String),
    Styled(RawStyledSidebarToken),
}

/// A token is a name or a style table. Written by hand, not as an untagged
/// enum: an untagged enum buffers the table, so one unknown key fails every
/// variant and the error loses the key's name (fork issue 162). Here an
/// unknown key is skipped through `IgnoredAny`, which the config loader
/// reports by path ("unknown config key ...rows.1.2.wobble"). A wrong value
/// type for a style key is dropped too, and reported by position through
/// `sidebar_style_diagnostics` (fork issue 163), so one typo does not cost the
/// file; only `token` itself, which names what the cell shows, must be right.
impl<'de> Deserialize<'de> for RawSidebarToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct TokenVisitor;

        impl<'de> serde::de::Visitor<'de> for TokenVisitor {
            type Value = RawSidebarToken;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(
                    "a token name or a { token, fg, bold, dim, italic, keep, truncate } table",
                )
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(RawSidebarToken::Plain(value.to_string()))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut token = None;
                let mut style = RawStyledSidebarToken {
                    token: String::new(),
                    fg: None,
                    bold: None,
                    dim: None,
                    italic: None,
                    keep: None,
                    truncate: None,
                    invalid: 0,
                };
                // A wrong-typed value reads as unset and sets the key's bit.
                fn lenient<'de, A, T>(
                    map: &mut A,
                    bit: u8,
                    invalid: &mut u8,
                ) -> Result<Option<T>, A::Error>
                where
                    A: serde::de::MapAccess<'de>,
                    T: serde::de::DeserializeOwned,
                {
                    let value = map.next_value::<toml::Value>()?;
                    match T::deserialize(value) {
                        Ok(value) => Ok(Some(value)),
                        Err(_) => {
                            *invalid |= bit;
                            Ok(None)
                        }
                    }
                }
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "token" => token = Some(map.next_value::<String>()?),
                        "fg" => style.fg = lenient(&mut map, STYLE_KEY_FG, &mut style.invalid)?,
                        "bold" => {
                            style.bold = lenient(&mut map, STYLE_KEY_BOLD, &mut style.invalid)?
                        }
                        "dim" => style.dim = lenient(&mut map, STYLE_KEY_DIM, &mut style.invalid)?,
                        "italic" => {
                            style.italic = lenient(&mut map, STYLE_KEY_ITALIC, &mut style.invalid)?
                        }
                        "keep" => {
                            style.keep = lenient(&mut map, STYLE_KEY_KEEP, &mut style.invalid)?
                        }
                        "truncate" => {
                            style.truncate = lenient::<_, String>(
                                &mut map,
                                STYLE_KEY_TRUNCATE,
                                &mut style.invalid,
                            )?
                            .map(|value| match value.as_str() {
                                "start" => SidebarTokenTruncate::Start,
                                "end" => SidebarTokenTruncate::End,
                                _ => SidebarTokenTruncate::Unrecognized,
                            });
                        }
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                style.token = token.ok_or_else(|| serde::de::Error::missing_field("token"))?;
                Ok(RawSidebarToken::Styled(style))
            }
        }

        deserializer.deserialize_any(TokenVisitor)
    }
}

impl RawSidebarToken {
    fn parts(self) -> (String, Option<SidebarTokenStyle>) {
        match self {
            Self::Plain(token) => (token, None),
            Self::Styled(token) => (
                token.token,
                Some(SidebarTokenStyle {
                    fg: token.fg,
                    bold: token.bold,
                    dim: token.dim,
                    italic: token.italic,
                    keep: token.keep,
                    truncate: token.truncate,
                    invalid: token.invalid,
                }),
            ),
        }
    }
}

fn parse_sidebar_token<T>(value: String, builtins: &[(&str, T)]) -> Result<T, String>
where
    T: Clone + From<String>,
{
    if let Some((_, token)) = builtins.iter().find(|(name, _)| *name == value) {
        return Ok(token.clone());
    }
    let Some(name) = value.strip_prefix('$') else {
        return Err(format!(
            "unknown sidebar token `{value}`; custom tokens must start with `$`"
        ));
    };
    if name.is_empty()
        || name.len() > 32
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return Err(format!("invalid custom sidebar token `{value}`"));
    }
    Ok(T::from(name.to_string()))
}

fn serialize_styled_token<S>(
    name: String,
    style: SidebarTokenStyle,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(None)?;
    map.serialize_entry("token", &name)?;
    if let Some(fg) = style.fg {
        map.serialize_entry("fg", &fg)?;
    }
    if let Some(bold) = style.bold {
        map.serialize_entry("bold", &bold)?;
    }
    if let Some(dim) = style.dim {
        map.serialize_entry("dim", &dim)?;
    }
    if let Some(italic) = style.italic {
        map.serialize_entry("italic", &italic)?;
    }
    if let Some(keep) = style.keep {
        map.serialize_entry("keep", &keep)?;
    }
    match style.truncate {
        Some(SidebarTokenTruncate::Start) => map.serialize_entry("truncate", "start")?,
        Some(SidebarTokenTruncate::End) => map.serialize_entry("truncate", "end")?,
        Some(SidebarTokenTruncate::Unrecognized) | None => {}
    }
    map.end()
}

fn agent_token_name(token: &AgentSidebarToken) -> String {
    match token {
        AgentSidebarToken::StateIcon => "state_icon".into(),
        AgentSidebarToken::StateText => "state_text".into(),
        AgentSidebarToken::Workspace => "workspace".into(),
        AgentSidebarToken::Tab => "tab".into(),
        AgentSidebarToken::Pane => "pane".into(),
        AgentSidebarToken::Agent => "agent".into(),
        AgentSidebarToken::TerminalTitle => "terminal_title".into(),
        AgentSidebarToken::TerminalTitleStripped => "terminal_title_stripped".into(),
        AgentSidebarToken::Custom(name) => format!("${name}"),
        AgentSidebarToken::Styled { token, .. } => agent_token_name(token),
    }
}

fn space_token_name(token: &SpaceSidebarToken) -> String {
    match token {
        SpaceSidebarToken::StateIcon => "state_icon".into(),
        SpaceSidebarToken::StateText => "state_text".into(),
        SpaceSidebarToken::Workspace => "workspace".into(),
        SpaceSidebarToken::Branch => "branch".into(),
        SpaceSidebarToken::GitStatus => "git_status".into(),
        SpaceSidebarToken::Custom(name) => format!("${name}"),
        SpaceSidebarToken::Styled { token, .. } => space_token_name(token),
    }
}

impl Serialize for AgentSidebarToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Styled { token, style } => {
                serialize_styled_token(agent_token_name(token), *style, serializer)
            }
            token => serializer.serialize_str(&agent_token_name(token)),
        }
    }
}

impl From<String> for AgentSidebarToken {
    fn from(value: String) -> Self {
        Self::Custom(value)
    }
}

impl<'de> Deserialize<'de> for AgentSidebarToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let (value, style) = RawSidebarToken::deserialize(deserializer)?.parts();
        let token = parse_sidebar_token(
            value,
            &[
                ("state_icon", Self::StateIcon),
                ("state_text", Self::StateText),
                ("workspace", Self::Workspace),
                ("tab", Self::Tab),
                ("pane", Self::Pane),
                ("agent", Self::Agent),
                ("terminal_title", Self::TerminalTitle),
                ("terminal_title_stripped", Self::TerminalTitleStripped),
            ],
        )
        .map_err(serde::de::Error::custom)?;
        Ok(style.map_or(token.clone(), |style| Self::Styled {
            token: Box::new(token),
            style,
        }))
    }
}

impl Serialize for SpaceSidebarToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Styled { token, style } => {
                serialize_styled_token(space_token_name(token), *style, serializer)
            }
            token => serializer.serialize_str(&space_token_name(token)),
        }
    }
}

impl From<String> for SpaceSidebarToken {
    fn from(value: String) -> Self {
        Self::Custom(value)
    }
}

impl<'de> Deserialize<'de> for SpaceSidebarToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let (value, style) = RawSidebarToken::deserialize(deserializer)?.parts();
        let token = parse_sidebar_token(
            value,
            &[
                ("state_icon", Self::StateIcon),
                ("state_text", Self::StateText),
                ("workspace", Self::Workspace),
                ("branch", Self::Branch),
                ("git_status", Self::GitStatus),
            ],
        )
        .map_err(serde::de::Error::custom)?;
        Ok(style.map_or(token.clone(), |style| Self::Styled {
            token: Box::new(token),
            style,
        }))
    }
}

type AgentSidebarRows = Vec<Vec<AgentSidebarToken>>;
type SpaceSidebarRows = Vec<Vec<SpaceSidebarToken>>;

fn deserialize_rows_by_agent<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, AgentSidebarRows>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let rows_by_agent = BTreeMap::<String, AgentSidebarRows>::deserialize(deserializer)?;
    for (id, rows) in &rows_by_agent {
        if crate::detect::parse_canonical_agent_label(id).is_none() {
            return Err(serde::de::Error::custom(format!(
                "unknown canonical agent id `{id}` in sidebar rows_by_agent"
            )));
        }
        validate_sidebar_rows(rows).map_err(serde::de::Error::custom)?;
    }
    Ok(rows_by_agent)
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentsSidebarConfig {
    #[serde(deserialize_with = "deserialize_sidebar_rows")]
    pub rows: AgentSidebarRows,
    #[serde(default, deserialize_with = "deserialize_rows_by_agent")]
    pub rows_by_agent: BTreeMap<String, AgentSidebarRows>,
    pub row_gap: u16,
}

impl AgentsSidebarConfig {
    pub(crate) fn rows_for_agent(&self, agent: Option<Agent>) -> &AgentSidebarRows {
        agent
            .and_then(|agent| self.rows_by_agent.get(crate::detect::agent_label(agent)))
            .unwrap_or(&self.rows)
    }
}

impl Default for AgentsSidebarConfig {
    fn default() -> Self {
        Self {
            rows: vec![
                vec![
                    AgentSidebarToken::StateIcon,
                    AgentSidebarToken::Workspace,
                    AgentSidebarToken::Tab,
                ],
                vec![AgentSidebarToken::Agent],
            ],
            rows_by_agent: BTreeMap::new(),
            row_gap: DEFAULT_SIDEBAR_ROW_GAP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SpacesSidebarConfig {
    #[serde(deserialize_with = "deserialize_sidebar_rows")]
    pub rows: SpaceSidebarRows,
    pub row_gap: u16,
}

impl Default for SpacesSidebarConfig {
    fn default() -> Self {
        Self {
            rows: vec![
                vec![SpaceSidebarToken::StateIcon, SpaceSidebarToken::Workspace],
                vec![SpaceSidebarToken::Branch, SpaceSidebarToken::GitStatus],
            ],
            row_gap: DEFAULT_SIDEBAR_ROW_GAP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct SidebarConfig {
    pub agents: AgentsSidebarConfig,
    pub spaces: SpacesSidebarConfig,
}

/// A `truncate` value this build does not know falls back to `end`, with one
/// line per occurrence (fork issue 164). Like an unknown key, it costs the
/// option and not the file.
pub(crate) fn sidebar_style_diagnostics(sidebar: &SidebarConfig) -> Vec<String> {
    fn unrecognized<T>(
        label: &str,
        rows: &[Vec<T>],
        parts: impl Fn(&T) -> SidebarTokenStyle,
    ) -> Vec<String> {
        let mut out = Vec::new();
        for (row, tokens) in rows.iter().enumerate() {
            for (column, token) in tokens.iter().enumerate() {
                let style = parts(token);
                if style.truncate == Some(SidebarTokenTruncate::Unrecognized) {
                    out.push(format!(
                        "{label}[{row}][{column}] has an unrecognized truncate value (expected \"start\" or \"end\"); using \"end\""
                    ));
                }
                for (key, expected) in style.invalid_keys() {
                    out.push(format!(
                        "{label}[{row}][{column}] has a wrong value type for `{key}` (expected {expected}); ignoring it"
                    ));
                }
            }
        }
        out
    }
    let mut out = unrecognized("ui.sidebar.agents.rows", &sidebar.agents.rows, |t| {
        t.parts().1
    });
    for (agent, rows) in &sidebar.agents.rows_by_agent {
        out.extend(unrecognized(
            &format!("ui.sidebar.agents.rows_by_agent.{agent}"),
            rows,
            |t| t.parts().1,
        ));
    }
    out.extend(unrecognized(
        "ui.sidebar.spaces.rows",
        &sidebar.spaces.rows,
        |t| t.parts().1,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_compact_agent_and_existing_space_layouts() {
        let config = SidebarConfig::default();
        assert_eq!(
            config.agents.rows,
            vec![
                vec![
                    AgentSidebarToken::StateIcon,
                    AgentSidebarToken::Workspace,
                    AgentSidebarToken::Tab,
                ],
                vec![AgentSidebarToken::Agent],
            ]
        );
        assert!(config.agents.rows_by_agent.is_empty());
        assert_eq!(config.agents.row_gap, 0);
        assert_eq!(
            config.spaces.rows,
            vec![
                vec![SpaceSidebarToken::StateIcon, SpaceSidebarToken::Workspace],
                vec![SpaceSidebarToken::Branch, SpaceSidebarToken::GitStatus],
            ]
        );
        assert_eq!(config.spaces.row_gap, 0);
    }

    #[test]
    fn parses_builtin_and_arbitrary_custom_tokens() {
        let config: crate::config::Config = toml::from_str(
            r#"
[ui.sidebar.agents]
rows = [["state_icon", "workspace"], ["state_text", "agent", "$summary"], ["terminal_title", "terminal_title_stripped", "$terminal_title"]]
row_gap = 1

[ui.sidebar.agents.rows_by_agent]
claude = [["terminal_title_stripped"], ["agent", "$model"]]

[ui.sidebar.spaces]
rows = [["workspace"], ["$jj_status"]]
row_gap = 3
"#,
        )
        .expect("sidebar token config");

        assert_eq!(
            config.ui.sidebar.agents.rows[1],
            vec![
                AgentSidebarToken::StateText,
                AgentSidebarToken::Agent,
                AgentSidebarToken::Custom("summary".into()),
            ]
        );
        assert_eq!(
            config.ui.sidebar.agents.rows[2],
            vec![
                AgentSidebarToken::TerminalTitle,
                AgentSidebarToken::TerminalTitleStripped,
                AgentSidebarToken::Custom("terminal_title".into()),
            ]
        );
        assert_eq!(
            config.ui.sidebar.agents.rows_by_agent["claude"],
            vec![
                vec![AgentSidebarToken::TerminalTitleStripped],
                vec![
                    AgentSidebarToken::Agent,
                    AgentSidebarToken::Custom("model".into()),
                ],
            ]
        );
        assert_eq!(config.ui.sidebar.agents.row_gap, 1);
        assert_eq!(
            config.ui.sidebar.spaces.rows[1],
            vec![SpaceSidebarToken::Custom("jj_status".into())]
        );
        assert_eq!(config.ui.sidebar.spaces.row_gap, 3);
    }

    #[test]
    fn parses_occurrence_styles_without_changing_plain_tokens() {
        let config: crate::config::Config = toml::from_str(
            r##"
[ui.sidebar.agents]
rows = [[{ token = "workspace", fg = "#abc", bold = false }, "workspace"], [{ token = "$summary", dim = false }]]

[ui.sidebar.agents.rows_by_agent]
claude = [[{ token = "agent", fg = "#112233", bold = true, dim = false, italic = true }]]

[ui.sidebar.spaces]
rows = [[{ token = "git_status", fg = "#ff00aa" }], [{ token = "$jj", bold = true, italic = false }]]
"##,
        )
        .unwrap();

        let (token, style) = config.ui.sidebar.agents.rows[0][0].parts();
        assert_eq!(token, &AgentSidebarToken::Workspace);
        assert_eq!(style.bold, Some(false));
        assert_eq!(
            style.fg.unwrap().ratatui(),
            ratatui::style::Color::Rgb(0xaa, 0xbb, 0xcc)
        );
        assert_eq!(
            config.ui.sidebar.agents.rows[0][1],
            AgentSidebarToken::Workspace
        );

        let (token, style) = config.ui.sidebar.agents.rows_by_agent["claude"][0][0].parts();
        assert_eq!(token, &AgentSidebarToken::Agent);
        assert_eq!(style.bold, Some(true));
        assert_eq!(style.dim, Some(false));
        assert_eq!(style.italic, Some(true));
        // Omitted stays omitted: contextual.
        assert_eq!(config.ui.sidebar.agents.rows[0][0].parts().1.italic, None);

        let (token, style) = config.ui.sidebar.spaces.rows[0][0].parts();
        assert_eq!(token, &SpaceSidebarToken::GitStatus);
        assert_eq!(
            style.fg.unwrap().ratatui(),
            ratatui::style::Color::Rgb(0xff, 0x00, 0xaa)
        );
        let (token, style) = config.ui.sidebar.spaces.rows[1][0].parts();
        assert_eq!(token, &SpaceSidebarToken::Custom("jj".into()));
        assert_eq!(style.bold, Some(true));
        assert_eq!(style.italic, Some(false));
    }

    #[test]
    fn keep_and_truncate_parse_and_round_trip() {
        let config: crate::config::Config = toml::from_str(
            r##"[ui.sidebar.spaces]
rows = [[{ token = "$asks", keep = true, truncate = "start" }, { token = "branch", keep = false, truncate = "end" }, "workspace"]]
"##,
        )
        .unwrap();
        let rows = &config.ui.sidebar.spaces.rows;
        let (_, first) = rows[0][0].parts();
        assert_eq!(first.keep, Some(true));
        assert_eq!(first.truncate, Some(SidebarTokenTruncate::Start));
        assert!(first.keeps_width() && first.truncates_start());
        let (_, second) = rows[0][1].parts();
        assert_eq!(second.keep, Some(false));
        assert_eq!(second.truncate, Some(SidebarTokenTruncate::End));
        assert!(!second.keeps_width() && !second.truncates_start());
        let (_, plain) = rows[0][2].parts();
        assert_eq!((plain.keep, plain.truncate), (None, None));

        let text = toml::to_string(&config.ui.sidebar.spaces).unwrap();
        let again: SpacesSidebarConfig = toml::from_str(&text).unwrap();
        assert_eq!(again.rows, rows.clone());
    }

    #[test]
    fn a_wrong_type_for_keep_or_truncate_is_an_error() {
        for entry in [
            r##"{ token = "workspace", keep = "yes" }"##,
            r##"{ token = "workspace", truncate = 1 }"##,
            r##"{ token = "workspace", truncate = true }"##,
        ] {
            let input = format!("[ui.sidebar.agents]\nrows = [[{entry}]]\n");
            assert!(
                toml::from_str::<crate::config::Config>(&input).is_err(),
                "accepted {entry}"
            );
        }
    }

    #[test]
    fn an_unrecognized_truncate_value_falls_back_to_end_with_a_diagnostic() {
        let config: crate::config::Config = toml::from_str(
            r##"[ui.sidebar.spaces]
rows = [["workspace"], [{ token = "$asks", truncate = "middle", keep = true }]]
[ui.sidebar.agents.rows_by_agent]
claude = [[{ token = "agent", truncate = "sideways" }]]
"##,
        )
        .unwrap();
        let (_, style) = config.ui.sidebar.spaces.rows[1][0].parts();
        assert_eq!(style.truncate, Some(SidebarTokenTruncate::Unrecognized));
        assert!(!style.truncates_start());
        assert_eq!(style.keep, Some(true), "the other options stay");
        assert_eq!(
            sidebar_style_diagnostics(&config.ui.sidebar),
            vec![
                "ui.sidebar.agents.rows_by_agent.claude[0][0] has an unrecognized truncate value (expected \"start\" or \"end\"); using \"end\"",
                "ui.sidebar.spaces.rows[1][0] has an unrecognized truncate value (expected \"start\" or \"end\"); using \"end\"",
            ]
        );
        assert!(sidebar_style_diagnostics(&SidebarConfig::default()).is_empty());
    }

    #[test]
    fn an_unknown_style_key_is_skipped_and_the_known_fields_stay() {
        let config: crate::config::Config = toml::from_str(
            r##"[ui.sidebar.spaces]
rows = [[{ token = "workspace", underline = true, bold = true }]]
"##,
        )
        .unwrap();
        let (token, style) = config.ui.sidebar.spaces.rows[0][0].parts();
        assert_eq!(token, &SpaceSidebarToken::Workspace);
        assert_eq!(style.bold, Some(true));
    }

    #[test]
    fn italic_round_trips_through_serialization() {
        let config: crate::config::Config = toml::from_str(
            r##"[ui.sidebar.spaces]
rows = [[{ token = "$asks", fg = "#ffd60a", bold = true, italic = true }, { token = "branch", italic = false }, "workspace"]]
"##,
        )
        .unwrap();
        let text = toml::to_string(&config.ui.sidebar.spaces).unwrap();
        let again: crate::config::SpacesSidebarConfig = toml::from_str(&text).unwrap();
        assert_eq!(again.rows, config.ui.sidebar.spaces.rows);
        assert_eq!(again.rows[0][0].parts().1.italic, Some(true));
        assert_eq!(again.rows[0][1].parts().1.italic, Some(false));
        assert_eq!(again.rows[0][2].parts().1.italic, None);
    }

    #[test]
    fn rejects_invalid_occurrence_styles() {
        for entry in [
            r##"{ token = "workspace", fg = "red" }"##,
            r##"{ token = "workspace", fg = "#abcd" }"##,
            r##"{ token = "workspace", bold = "yes" }"##,
            r##"{ token = "workspace", italic = 1 }"##,
            r##"{ fg = "#abc" }"##,
        ] {
            let input = format!("[ui.sidebar.agents]\nrows = [[{entry}]]\n");
            assert!(
                toml::from_str::<crate::config::Config>(&input).is_err(),
                "accepted {entry}"
            );
        }
    }

    #[test]
    fn rejects_unknown_bare_and_malformed_custom_tokens() {
        for token in ["summary", "$", "$bad.name"] {
            let input = format!("[ui.sidebar.agents]\\nrows = [[\"{token}\"]]\\n");
            assert!(toml::from_str::<crate::config::Config>(&input).is_err());
        }
    }

    #[test]
    fn rejects_oversized_sidebar_layouts() {
        let too_many_rows = std::iter::repeat_n("[\"agent\"]", MAX_SIDEBAR_ROWS + 1)
            .collect::<Vec<_>>()
            .join(",");
        let input = format!("[ui.sidebar.agents]\nrows = [{too_many_rows}]\n");
        assert!(toml::from_str::<crate::config::Config>(&input).is_err());

        let too_many_tokens = std::iter::repeat_n("\"workspace\"", MAX_SIDEBAR_TOKENS_PER_ROW + 1)
            .collect::<Vec<_>>()
            .join(",");
        let input = format!("[ui.sidebar.spaces]\nrows = [[{too_many_tokens}]]\n");
        assert!(toml::from_str::<crate::config::Config>(&input).is_err());

        let input = format!("[ui.sidebar.agents.rows_by_agent]\nclaude = [{too_many_rows}]\n");
        assert!(toml::from_str::<crate::config::Config>(&input).is_err());
    }

    #[test]
    fn accepts_every_canonical_agent_override_key() {
        let agents = [
            Agent::Pi,
            Agent::Claude,
            Agent::Codex,
            Agent::Gemini,
            Agent::Cursor,
            Agent::Devin,
            Agent::Antigravity,
            Agent::Cline,
            Agent::Omp,
            Agent::Mastracode,
            Agent::OpenCode,
            Agent::GithubCopilot,
            Agent::Kimi,
            Agent::Kiro,
            Agent::Droid,
            Agent::Amp,
            Agent::Grok,
            Agent::Hermes,
            Agent::Kilo,
            Agent::Qodercli,
            Agent::Qwen,
            Agent::Maki,
        ];
        let entries = agents
            .iter()
            .map(|agent| format!("{} = [[\"agent\"]]", crate::detect::agent_label(*agent)))
            .collect::<Vec<_>>()
            .join("\n");
        let input = format!("[ui.sidebar.agents.rows_by_agent]\n{entries}\n");
        let config: crate::config::Config = toml::from_str(&input).expect("canonical keys");

        assert_eq!(config.ui.sidebar.agents.rows_by_agent.len(), agents.len());
    }

    #[test]
    fn rejects_alias_case_whitespace_and_unknown_override_keys() {
        for key in ["claude-code", "Claude", "' claude '", "unknown"] {
            let input = format!("[ui.sidebar.agents.rows_by_agent]\n{key} = [[\"agent\"]]\n");
            assert!(
                toml::from_str::<crate::config::Config>(&input).is_err(),
                "accepted key {key:?}"
            );
        }
    }
}
