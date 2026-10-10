use std::path::{Path, PathBuf};

use tracing::warn;

use super::{model::LoadedConfig, Config, CONFIG_PATH_ENV_VAR};

const KNOWN_TOP_LEVEL_CONFIG_KEYS: &[&str] = &[
    "advanced",
    "agents",
    "experimental",
    "keys",
    "onboarding",
    "remote",
    "server",
    "session",
    "terminal",
    "theme",
    "ui",
    "update",
    "worktrees",
];

pub fn app_dir_name() -> &'static str {
    if cfg!(debug_assertions) {
        "herdr-dev"
    } else {
        "herdr"
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(dir) = xdg_dir("XDG_CONFIG_HOME") {
        return dir.join(app_dir_name());
    }
    default_config_dir()
}

pub fn state_dir() -> PathBuf {
    if let Some(dir) = xdg_dir("XDG_STATE_HOME") {
        return dir.join(app_dir_name());
    }
    default_state_dir()
}

/// A path-valued environment variable.
///
/// A unit-test binary does not honor an absolute path outside the scratch area
/// (see [`is_unit_test_scratch`]): that is the developer's own
/// `HERDR_SOCKET_PATH`, `CLAUDE_CONFIG_DIR` and the like, and following it would
/// read or rewrite state of a herdr or agent running on the machine that runs
/// the tests. A test that needs a path points the variable into a temp
/// directory, which is honored.
pub(crate) fn path_env(name: &str) -> Option<std::ffi::OsString> {
    let value = std::env::var_os(name)?;
    #[cfg(test)]
    if std::path::Path::new(&value).is_absolute() && !is_unit_test_scratch(Path::new(&value)) {
        return None;
    }
    Some(value)
}

/// The user's home directory from `HOME`.
///
/// A unit-test binary only honors a `HOME` inside the scratch area; the
/// developer's own gets a private stand-in (never created), so a test cannot
/// read their dotfiles, agent configs, ssh config or transcripts.
pub(crate) fn home_env() -> Option<std::ffi::OsString> {
    let home = std::env::var_os("HOME")?;
    #[cfg(test)]
    if !is_unit_test_scratch(Path::new(&home)) {
        return Some(unit_test_host_dir("home").into_os_string());
    }
    Some(home)
}

/// The XDG base directory named by `var`.
///
/// A unit-test binary only honors one that points into a scratch area (see
/// [`is_unit_test_scratch`]). The value a developer's shell exports for the
/// machine's real config or state directory is ignored, so a test that has not
/// pointed the variable at its own directory still cannot read or overwrite the
/// config, session, release notes or manifest cache of a herdr running on the
/// same machine.
fn xdg_dir(var: &str) -> Option<PathBuf> {
    path_env(var)?.into_string().ok().map(PathBuf::from)
}

/// Where herdr keeps its config when no XDG base directory applies.
fn default_config_dir() -> PathBuf {
    #[cfg(test)]
    return unit_test_host_dir("config");
    #[cfg(not(test))]
    platform_config_dir()
}

/// The state-directory counterpart of [`default_config_dir`].
fn default_state_dir() -> PathBuf {
    #[cfg(test)]
    return unit_test_host_dir("state");
    #[cfg(not(test))]
    platform_state_dir()
}

/// Whether `path` lies in the temporary area unit tests are allowed to use.
#[cfg(test)]
pub(crate) fn is_unit_test_scratch(path: &Path) -> bool {
    [
        std::env::temp_dir().as_path(),
        Path::new("/tmp"),
        Path::new("/private/tmp"),
        Path::new("/var/folders"),
        Path::new("/private/var/folders"),
    ]
    .iter()
    .any(|root| path.starts_with(root))
}

/// A per-process directory that is only named here, never created; the first
/// test that writes into it creates it. Kept short because Unix socket paths
/// are limited to roughly 100 bytes and tests bind sockets below the config
/// directory.
#[cfg(test)]
pub(crate) fn unit_test_host_dir(kind: &str) -> PathBuf {
    let base = if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    base.join(format!("herdr-ut-{}", std::process::id()))
        .join(kind)
        .join(app_dir_name())
}

#[cfg(all(windows, not(test)))]
fn platform_config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("APPDATA") {
        return PathBuf::from(dir).join(app_dir_name());
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        return PathBuf::from(profile)
            .join("AppData")
            .join("Roaming")
            .join(app_dir_name());
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(format!(".config/{}", app_dir_name()));
    }
    std::env::temp_dir().join(app_dir_name())
}

#[cfg(all(not(windows), not(test)))]
fn platform_config_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(format!(".config/{}", app_dir_name()))
    } else {
        std::env::temp_dir().join(app_dir_name())
    }
}

#[cfg(all(windows, not(test)))]
fn platform_state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(dir).join(app_dir_name());
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        return PathBuf::from(profile)
            .join("AppData")
            .join("Local")
            .join(app_dir_name());
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(format!(".local/state/{}", app_dir_name()));
    }
    std::env::temp_dir().join(format!("{}-state", app_dir_name()))
}

#[cfg(all(not(windows), not(test)))]
fn platform_state_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(format!(".local/state/{}", app_dir_name()))
    } else {
        std::env::temp_dir().join(format!("{}-state", app_dir_name()))
    }
}

/// Normalize UTF-8 byte-order marks in config text.
///
/// TOML tolerates a single BOM at the very start of the document, but a BOM at
/// the start of a later line makes the parser reject the whole file. A
/// line-oriented edit can displace a leading BOM into the middle of the file,
/// so drop line-start BOMs that the TOML parser actually rejects. A U+FEFF that
/// is valid string data is kept, because its parse error would not point at it.
fn normalize_utf8_bom(content: &str) -> String {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    if !content.contains('\u{feff}') {
        return content.to_owned();
    }

    let mut normalized = content.to_owned();
    while let Err(error) = normalized.parse::<toml::Value>() {
        let Some(span) = error.span() else {
            break;
        };
        if normalized.get(span.clone()) != Some("\u{feff}") {
            break;
        }
        normalized.replace_range(span, "");
    }
    normalized
}

pub(super) fn read_optional_config(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(normalize_utf8_bom(&content))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

impl Config {
    pub fn load() -> LoadedConfig {
        let path = config_path();
        let content = match read_optional_config(&path) {
            Ok(Some(content)) => content,
            Ok(None) => {
                return LoadedConfig {
                    config: Self::default(),
                    diagnostics: Vec::new(),
                    invalid_sections: Vec::new(),
                };
            }
            Err(err) => {
                warn!(err = %err, "config read error, using defaults");
                return LoadedConfig {
                    config: Self::default(),
                    diagnostics: vec![format!("config read error: {err}; using defaults")],
                    invalid_sections: Vec::new(),
                };
            }
        };

        // A value this build does not know (a newer build's enum variant, a
        // typo) costs that one key, not the file (fork issue 133).
        let (content, mut repair_diagnostics) = repair_invalid_values(&content);

        match deserialize_with_ignored::<Config, _>(toml::Deserializer::new(&content)) {
            Ok((config, ignored_keys)) => {
                let (unknown_sections, mut diagnostics) =
                    unknown_top_level_sections_from_str(&content);
                diagnostics.append(&mut repair_diagnostics);
                diagnostics.extend(unknown_config_key_diagnostics(
                    ignored_keys
                        .into_iter()
                        .filter(|path| {
                            !matches!(path.as_slice(), [ConfigKeyPathSegment::Key(key)] if unknown_sections.contains(key))
                        })
                        .collect(),
                    None,
                ));
                diagnostics.extend(config.collect_diagnostics());
                LoadedConfig {
                    config,
                    diagnostics,
                    invalid_sections: Vec::new(),
                }
            }
            Err(err) => {
                warn!(err = %err, "config parse error, using defaults");
                LoadedConfig {
                    config: Self::default(),
                    diagnostics: vec![format!("config parse error: {err}; using defaults")],
                    invalid_sections: Vec::new(),
                }
            }
        }
    }
}

pub(super) fn resolve_config_relative_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    config_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path)
}

pub fn config_path() -> PathBuf {
    if let Some(path) = path_env(CONFIG_PATH_ENV_VAR).and_then(|path| path.into_string().ok()) {
        return PathBuf::from(path);
    }
    config_dir().join("config.toml")
}

pub fn config_diagnostic_summary(diagnostics: &[String]) -> Option<String> {
    if diagnostics.is_empty() {
        return None;
    }

    let target = config_path()
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml")
        .to_string();
    let read_error = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.starts_with("config read error:"));
    let impact = if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("using defaults"))
    {
        if read_error {
            " unreadable; using defaults"
        } else {
            " invalid; using defaults"
        }
    } else if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("keeping current config"))
    {
        if read_error {
            " unreadable; keeping current config"
        } else {
            " invalid; keeping current config"
        }
    } else if diagnostics
        .iter()
        .all(|diagnostic| diagnostic.starts_with("unknown config key "))
    {
        " has unknown keys"
    } else {
        ""
    };

    Some(format!("{target}{impact}; herdr config check"))
}

pub fn load_live_config() -> Result<LoadedConfig, Vec<String>> {
    let path = config_path();
    let content = match read_optional_config(&path) {
        Ok(Some(content)) => content,
        Ok(None) => {
            return Ok(LoadedConfig {
                config: Config::default(),
                diagnostics: Vec::new(),
                invalid_sections: Vec::new(),
            });
        }
        Err(err) => {
            return Err(vec![format!(
                "config read error: {err}; keeping current config"
            )]);
        }
    };
    load_live_config_from_str(&content)
}

fn load_live_config_from_str(content: &str) -> Result<LoadedConfig, Vec<String>> {
    let (content, repair_diagnostics) = repair_invalid_values(content);
    let content = content.as_str();
    let value = content
        .parse::<toml::Value>()
        .map_err(|err| vec![format!("config parse error: {err}; keeping current config")])?;
    let table = value.as_table().ok_or_else(|| {
        vec![
            "config parse error: top-level config must be a table; keeping current config"
                .to_string(),
        ]
    })?;

    let mut config = Config::default();
    let mut diagnostics = repair_diagnostics;
    diagnostics.extend(unknown_top_level_section_diagnostics(table));
    diagnostics.extend(unknown_top_level_config_key_diagnostics(table));
    let mut invalid_sections = Vec::new();

    if let Some(value) = table.get("onboarding") {
        match value.clone().try_into::<Option<bool>>() {
            Ok(onboarding) => config.onboarding = onboarding,
            Err(err) => diagnostics.push(format!(
                "invalid onboarding setting: {err}; keeping current onboarding state"
            )),
        }
    }

    load_live_section(
        table,
        "theme",
        "theme config",
        &mut diagnostics,
        &mut invalid_sections,
        |section| config.theme = section,
    );
    load_live_section(
        table,
        "keys",
        "keybinding config",
        &mut diagnostics,
        &mut invalid_sections,
        |section| config.keys = section,
    );
    load_live_section(
        table,
        "terminal",
        "terminal config",
        &mut diagnostics,
        &mut invalid_sections,
        |section| config.terminal = section,
    );
    load_live_section(
        table,
        "session",
        "session config",
        &mut diagnostics,
        &mut invalid_sections,
        |section| config.session = section,
    );
    load_live_section(
        table,
        "server",
        "server config",
        &mut diagnostics,
        &mut invalid_sections,
        |section| config.server = section,
    );
    load_live_section(
        table,
        "update",
        "update config",
        &mut diagnostics,
        &mut invalid_sections,
        |section| config.update = section,
    );
    load_live_section(
        table,
        "ui",
        "ui config",
        &mut diagnostics,
        &mut invalid_sections,
        |section| config.ui = section,
    );
    load_live_section(
        table,
        "advanced",
        "advanced config",
        &mut diagnostics,
        &mut invalid_sections,
        |section| config.advanced = section,
    );
    load_live_section(
        table,
        "worktrees",
        "worktree config",
        &mut diagnostics,
        &mut invalid_sections,
        |section| config.worktrees = section,
    );
    load_live_section(
        table,
        "experimental",
        "experimental config",
        &mut diagnostics,
        &mut invalid_sections,
        |section| config.experimental = section,
    );
    load_live_section(
        table,
        "remote",
        "remote config",
        &mut diagnostics,
        &mut invalid_sections,
        |section| config.remote = section,
    );
    load_live_section(
        table,
        "agents",
        "agents config",
        &mut diagnostics,
        &mut invalid_sections,
        |section| config.agents = section,
    );

    diagnostics.extend(config.theme.diagnostics());

    Ok(LoadedConfig {
        config,
        diagnostics,
        invalid_sections,
    })
}

/// How many bad values one file may lose before the repair gives up.
const MAX_VALUE_REPAIRS: usize = 64;

/// Turn each value of valid TOML that the config model rejects (an unknown
/// enum variant, a wrong type) into a per-key warning, so that key falls back
/// to its default and the rest of the file applies (fork issue 133). The
/// offending `key = value` line is blanked, never deleted, so the line
/// numbers of the other findings stay true. Real TOML syntax errors, and
/// errors that do not point at a single `key = value` line, are left for the
/// normal parse to report and reject as before.
fn repair_invalid_values(content: &str) -> (String, Vec<String>) {
    if content.parse::<toml::Table>().is_err() {
        return (content.to_string(), Vec::new());
    }
    let mut text = content.to_string();
    let mut diagnostics = Vec::new();
    for _ in 0..MAX_VALUE_REPAIRS {
        let Err(err) = deserialize_with_ignored::<Config, _>(toml::Deserializer::new(&text)) else {
            break;
        };
        let Some(span) = err.span() else { break };
        let Some((line_start, line_end)) = blank_target_line(&text, span.start) else {
            break;
        };
        let line = &text[line_start..line_end];
        let Some((key, _)) = line.split_once('=') else {
            break;
        };
        let key = key.trim();
        if key.is_empty() {
            break;
        }
        let path = match table_header_before(&text, line_start) {
            Some(header) => format!("{header}.{key}"),
            None => key.to_string(),
        };
        let line_number = text[..line_start].matches('\n').count() + 1;
        diagnostics.push(format!(
            "invalid value for {path} at line {line_number}: {}; using the default for this key",
            err.message()
        ));
        // Same byte length, so no later span moves.
        text.replace_range(line_start..line_end, &" ".repeat(line_end - line_start));
    }
    (text, diagnostics)
}

/// The byte range of the line that holds `offset`, when that line is one
/// `key = value` pair (not a table header or a comment).
fn blank_target_line(text: &str, offset: usize) -> Option<(usize, usize)> {
    if offset > text.len() {
        return None;
    }
    let start = text[..offset].rfind('\n').map_or(0, |at| at + 1);
    let end = text[offset..]
        .find('\n')
        .map_or(text.len(), |at| offset + at);
    let line = text[start..end].trim_start();
    (line.contains('=') && !line.starts_with('[') && !line.starts_with('#')).then_some((start, end))
}

/// The dotted name of the last `[table]` header above `offset`, if any.
fn table_header_before(text: &str, offset: usize) -> Option<String> {
    text[..offset].lines().rev().find_map(|line| {
        let line = line.trim();
        let inner = line.strip_prefix('[')?.strip_suffix(']')?;
        Some(inner.trim_matches(['[', ']']).trim().to_string())
    })
}

fn unknown_top_level_sections_from_str(content: &str) -> (Vec<String>, Vec<String>) {
    let Ok(value) = content.parse::<toml::Value>() else {
        return (Vec::new(), Vec::new());
    };
    let Some(table) = value.as_table() else {
        return (Vec::new(), Vec::new());
    };

    let mut keys = Vec::new();
    let mut diagnostics = Vec::new();
    for (key, value) in table {
        if let Some(diagnostic) = unknown_top_level_section_diagnostic(key, value) {
            keys.push(key.clone());
            diagnostics.push(diagnostic);
        }
    }
    (keys, diagnostics)
}

fn unknown_top_level_section_diagnostics(
    table: &toml::map::Map<String, toml::Value>,
) -> Vec<String> {
    table
        .iter()
        .filter_map(|(key, value)| unknown_top_level_section_diagnostic(key, value))
        .collect()
}

fn unknown_top_level_section_diagnostic(key: &str, value: &toml::Value) -> Option<String> {
    if KNOWN_TOP_LEVEL_CONFIG_KEYS.contains(&key) {
        return None;
    }

    let header = if value.is_table() {
        format!("[{key}]")
    } else if value
        .as_array()
        .is_some_and(|items| !items.is_empty() && items.iter().all(toml::Value::is_table))
    {
        format!("[[{key}]]")
    } else {
        return None;
    };

    if key == "toast" {
        Some(format!(
            "unknown config section {header}; did you mean [ui.toast]? ignoring section"
        ))
    } else {
        Some(format!("unknown config section {header}; ignoring section"))
    }
}

fn unknown_top_level_config_key_diagnostics(
    table: &toml::map::Map<String, toml::Value>,
) -> Vec<String> {
    let paths = table
        .iter()
        .filter(|(key, value)| {
            !KNOWN_TOP_LEVEL_CONFIG_KEYS.contains(&key.as_str())
                && unknown_top_level_section_diagnostic(key, value).is_none()
        })
        .map(|(key, _)| vec![ConfigKeyPathSegment::Key(key.clone())])
        .collect();
    unknown_config_key_diagnostics(paths, None)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ConfigKeyPathSegment {
    Key(String),
    Index(usize),
}

fn config_key_path(path: &serde_ignored::Path<'_>) -> Vec<ConfigKeyPathSegment> {
    fn visit(path: &serde_ignored::Path<'_>, segments: &mut Vec<ConfigKeyPathSegment>) {
        match path {
            serde_ignored::Path::Root => {}
            serde_ignored::Path::Seq { parent, index } => {
                visit(parent, segments);
                segments.push(ConfigKeyPathSegment::Index(*index));
            }
            serde_ignored::Path::Map { parent, key } => {
                visit(parent, segments);
                segments.push(ConfigKeyPathSegment::Key(key.clone()));
            }
            serde_ignored::Path::Some { parent }
            | serde_ignored::Path::NewtypeStruct { parent }
            | serde_ignored::Path::NewtypeVariant { parent } => visit(parent, segments),
        }
    }

    let mut segments = Vec::new();
    visit(path, &mut segments);
    segments
}

fn format_config_key_path(path: &[ConfigKeyPathSegment]) -> String {
    path.iter()
        .map(|segment| match segment {
            ConfigKeyPathSegment::Key(key)
                if !key.is_empty()
                    && key.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
                    }) =>
            {
                key.clone()
            }
            ConfigKeyPathSegment::Key(key) => toml::Value::String(key.clone()).to_string(),
            ConfigKeyPathSegment::Index(index) => index.to_string(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn unknown_config_key_diagnostics(
    paths: Vec<Vec<ConfigKeyPathSegment>>,
    section: Option<&str>,
) -> Vec<String> {
    let mut paths: Vec<Vec<ConfigKeyPathSegment>> = paths
        .into_iter()
        .map(|mut path| {
            if let Some(section) = section {
                path.insert(0, ConfigKeyPathSegment::Key(section.to_string()));
            }
            path
        })
        .collect();
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| {
            format!(
                "unknown config key {}; ignoring key",
                format_config_key_path(&path)
            )
        })
        .collect()
}

fn deserialize_with_ignored<'de, T, D>(
    deserializer: D,
) -> Result<(T, Vec<Vec<ConfigKeyPathSegment>>), D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    let mut ignored = Vec::new();
    let value = serde_ignored::deserialize(deserializer, |path| {
        ignored.push(config_key_path(&path));
    })?;
    Ok((value, ignored))
}

fn load_live_section<T>(
    table: &toml::map::Map<String, toml::Value>,
    section: &'static str,
    label: &str,
    diagnostics: &mut Vec<String>,
    invalid_sections: &mut Vec<String>,
    apply: impl FnOnce(T),
) where
    T: serde::de::DeserializeOwned,
{
    let Some(value) = table.get(section) else {
        return;
    };

    match deserialize_with_ignored(value.clone()) {
        Ok((section_config, ignored_keys)) => {
            diagnostics.extend(unknown_config_key_diagnostics(ignored_keys, Some(section)));
            apply(section_config);
        }
        Err(err) => {
            diagnostics.push(format!(
                "invalid {label}: {err}; keeping current {section} settings"
            ));
            invalid_sections.push(section.to_string());
        }
    }
}

pub(crate) fn upsert_top_level_bool(content: &str, key: &str, value: bool) -> String {
    let replacement = format!("{key} = {value}");
    let mut lines: Vec<String> = content.lines().map(|line| line.to_string()).collect();
    let mut in_section = false;

    for line in &mut lines {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = true;
            continue;
        }
        if in_section {
            continue;
        }
        if trimmed.starts_with(&format!("{key} ")) || trimmed.starts_with(&format!("{key}=")) {
            *line = replacement.clone();
            return lines.join("\n") + "\n";
        }
    }

    if lines.is_empty() {
        format!("{replacement}\n")
    } else {
        format!("{replacement}\n{}\n", lines.join("\n").trim_end())
    }
}

/// Write a key = value pair in a TOML section (creates section if missing).
pub fn upsert_section_value(content: &str, section: &str, key: &str, value: &str) -> String {
    upsert_section_raw(content, section, key, value)
}

pub fn upsert_section_bool(content: &str, section: &str, key: &str, value: bool) -> String {
    upsert_section_raw(content, section, key, &value.to_string())
}

pub fn remove_section_key(content: &str, section: &str, key: &str) -> String {
    let header = format!("[{section}]");
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;
    let mut in_section = false;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed == header;
            result.push(line.to_string());
            i += 1;
            continue;
        }

        if in_section
            && (trimmed.starts_with(&format!("{key} ")) || trimmed.starts_with(&format!("{key}=")))
        {
            i += 1;
            continue;
        }

        result.push(line.to_string());
        i += 1;
    }

    result.join("\n") + "\n"
}

pub fn remove_keybinding_config_sections(content: &str) -> (String, bool) {
    let mut result = Vec::new();
    let mut removed = false;
    let mut skipping_key_section = false;
    let mut in_table = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if let Some(table_name) = toml_table_header_name(trimmed) {
            in_table = true;
            skipping_key_section = is_keys_table_name(table_name);
            if skipping_key_section {
                removed = true;
                continue;
            }
        } else if skipping_key_section || (!in_table && is_top_level_keys_assignment(trimmed)) {
            removed = true;
            continue;
        }

        result.push(line.to_string());
    }

    let mut updated = result.join("\n");
    if content.ends_with('\n') || !updated.is_empty() {
        updated.push('\n');
    }
    (updated, removed)
}

fn toml_table_header_name(trimmed: &str) -> Option<&str> {
    if let Some(name) = trimmed
        .strip_prefix("[[")
        .and_then(|value| value.strip_suffix("]]"))
    {
        return Some(name.trim());
    }
    trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .map(str::trim)
}

fn is_keys_table_name(name: &str) -> bool {
    name == "keys" || name.starts_with("keys.")
}

fn is_top_level_keys_assignment(trimmed: &str) -> bool {
    trimmed.starts_with("keys ") || trimmed.starts_with("keys=") || trimmed.starts_with("keys.")
}

fn upsert_section_raw(content: &str, section: &str, key: &str, value: &str) -> String {
    let header = format!("[{section}]");
    let assignment = format!("{key} = {value}");
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;
    let mut found_section = false;
    let mut inserted = false;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed == header {
            found_section = true;
            result.push(line.to_string());
            i += 1;

            while i < lines.len() {
                let current = lines[i];
                let current_trimmed = current.trim();
                if current_trimmed.starts_with('[') && current_trimmed.ends_with(']') {
                    if !inserted {
                        result.push(assignment.clone());
                        inserted = true;
                    }
                    break;
                }

                if current_trimmed.starts_with(&format!("{key} "))
                    || current_trimmed.starts_with(&format!("{key}="))
                {
                    result.push(assignment.clone());
                    inserted = true;
                } else {
                    result.push(current.to_string());
                }
                i += 1;
            }

            continue;
        }

        result.push(line.to_string());
        i += 1;
    }

    if !found_section {
        if !result.is_empty() && !result.last().is_some_and(|line| line.trim().is_empty()) {
            result.push(String::new());
        }
        result.push(header);
        result.push(assignment);
    } else if !inserted {
        result.push(assignment);
    }

    result.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fork issue 133: one value this build does not know costs that key only.
    const CONFIG_WITH_ONE_BAD_VALUE: &str = "\
[keys]
prefix = \"ctrl+a\"

[theme]
name = \"nord\"

[ui]
sidebar_width = 31
workspace_sort = \"from-a-newer-build\"
agent_panel_sort = \"priority\"
";

    #[test]
    fn an_unknown_enum_value_is_a_per_key_warning_with_its_line() {
        let (repaired, diagnostics) = repair_invalid_values(CONFIG_WITH_ONE_BAD_VALUE);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(
            diagnostics[0].contains("ui.workspace_sort"),
            "{diagnostics:?}"
        );
        assert!(diagnostics[0].contains("line 9"), "{diagnostics:?}");
        assert!(
            diagnostics[0].contains("from-a-newer-build"),
            "{diagnostics:?}"
        );
        assert!(diagnostics[0].contains("default"), "{diagnostics:?}");
        // The rest of the file is untouched and still parses.
        let config: Config = toml::from_str(&repaired).expect("repaired file parses");
        assert_eq!(config.ui.sidebar_width, 31);
        assert_eq!(config.theme.name.as_deref(), Some("nord"));
        assert_eq!(
            repaired.len(),
            CONFIG_WITH_ONE_BAD_VALUE.len(),
            "lines keep their place"
        );
    }

    #[test]
    fn a_live_reload_keeps_every_other_key_and_section_when_one_value_is_bad() {
        let loaded = load_live_config_from_str(CONFIG_WITH_ONE_BAD_VALUE).expect("loads");
        assert_eq!(loaded.config.ui.sidebar_width, 31);
        assert_eq!(loaded.config.theme.name.as_deref(), Some("nord"));
        assert!(
            loaded
                .diagnostics
                .iter()
                .any(|d| d.contains("ui.workspace_sort") && d.contains("line 9")),
            "{:?}",
            loaded.diagnostics
        );
        assert!(loaded.invalid_sections.is_empty(), "no section was dropped");
    }

    #[test]
    fn several_bad_values_are_each_reported_and_the_default_applies() {
        let content =
            "[ui]\nworkspace_sort = \"x\"\nsidebar_width = 31\nagent_panel_sort = \"y\"\n";
        let (repaired, diagnostics) = repair_invalid_values(content);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
        let config: Config = toml::from_str(&repaired).unwrap();
        assert_eq!(config.ui.sidebar_width, 31);
    }

    // The report that opened fork issue 133: a newer build's variant in a
    // nested table dropped every keybinding, the theme and all of [ui].
    #[test]
    fn a_newer_builds_variant_in_a_nested_table_keeps_keys_theme_and_ui() {
        let content = "[keys]\nprefix = \"ctrl+a\"\n\n[theme]\nname = \"nord\"\n\n[ui]\nsidebar_width = 31\n\n[ui.toast.clipboard]\nenabled = true\nposition = \"from-a-newer-build\"\n";
        let loaded = load_live_config_from_str(content).expect("loads");
        assert_eq!(loaded.config.ui.sidebar_width, 31);
        assert_eq!(loaded.config.theme.name.as_deref(), Some("nord"));
        assert!(
            loaded.config.ui.toast.clipboard.enabled,
            "its sibling key applies"
        );
        assert!(
            loaded
                .diagnostics
                .iter()
                .any(|d| { d.contains("ui.toast.clipboard.position") && d.contains("line 12") }),
            "{:?}",
            loaded.diagnostics
        );
    }

    #[test]
    fn broken_toml_syntax_is_still_rejected_whole() {
        let content = "[ui\nworkspace_sort = \"x\"\n";
        let (repaired, diagnostics) = repair_invalid_values(content);
        assert_eq!(repaired, content);
        assert!(diagnostics.is_empty());
        assert!(load_live_config_from_str(content).is_err());
    }

    #[test]
    fn a_clean_file_has_nothing_to_repair() {
        let content = "[ui]\nsidebar_width = 31\n";
        let (repaired, diagnostics) = repair_invalid_values(content);
        assert_eq!(repaired, content);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn upsert_top_level_bool_replaces_existing_value() {
        let content = "onboarding = true\n[keys]\nprefix = \"ctrl+b\"\n";
        let updated = upsert_top_level_bool(content, "onboarding", false);
        assert!(updated.contains("onboarding = false"));
        assert!(!updated.contains("onboarding = true"));
    }

    #[test]
    fn upsert_section_bool_adds_missing_section() {
        let updated = upsert_section_bool("", "ui.toast", "enabled", true);
        assert!(updated.contains("[ui.toast]"));
        assert!(updated.contains("enabled = true"));
    }

    #[test]
    fn remove_section_key_removes_matching_key_from_section() {
        let content =
            "[ui.toast]\nenabled = true\ndelivery = \"herdr\"\n[ui.sound]\nenabled = true\n";
        let updated = remove_section_key(content, "ui.toast", "enabled");
        assert!(!updated.contains("[ui.toast]\nenabled = true"));
        assert!(updated.contains("delivery = \"herdr\""));
        assert!(updated.contains("[ui.sound]\nenabled = true"));
    }

    #[test]
    fn config_diagnostic_summary_uses_compact_actionable_banner() {
        let diagnostics = vec![
            "one".to_string(),
            "two".to_string(),
            "three".to_string(),
            "four".to_string(),
            "five".to_string(),
        ];

        assert_eq!(
            config_diagnostic_summary(&diagnostics).as_deref(),
            Some("config.toml; herdr config check")
        );
    }

    #[test]
    fn config_diagnostic_summary_reports_unknown_keys_compactly() {
        let diagnostics = vec![
            "unknown config key ui.mouse_captur; ignoring key".to_string(),
            "unknown config key keys.new_tabb; ignoring key".to_string(),
        ];

        assert_eq!(
            config_diagnostic_summary(&diagnostics).as_deref(),
            Some("config.toml has unknown keys; herdr config check")
        );
    }

    /// `KeysConfig` (serialize) and `KeysConfigOverlay` (deserialize) are
    /// separate structs, so an action wired into one but not the other fails
    /// silently: it appears in `--default-config` yet is rejected as an unknown
    /// key when a user actually sets it. This regressed for `next_layout` and
    /// `balance_panes` and stayed invisible until v0.7.5 added unknown-key
    /// reporting. Guard the two structs against drift: every `[keys]` action
    /// must deserialize through the overlay without an unknown-key diagnostic.
    #[test]
    fn every_keybinding_action_is_accepted_by_the_keys_overlay() {
        const MODEL_SRC: &str = include_str!("model.rs");
        let mut body = String::from("[keys]\n");
        let mut actions = Vec::new();
        for line in MODEL_SRC.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("pub ") else {
                continue;
            };
            let Some((name, ty)) = rest.split_once(':') else {
                continue;
            };
            if ty.trim().trim_end_matches(',') != "BindingConfig" {
                continue;
            }
            let name = name.trim();
            body.push_str(&format!("{name} = \"prefix+f1\"\n"));
            actions.push(name.to_string());
        }
        assert!(
            !actions.is_empty(),
            "no `pub <field>: BindingConfig` actions found in config/model.rs"
        );

        let (_config, ignored) =
            deserialize_with_ignored::<Config, _>(toml::Deserializer::new(&body))
                .expect("a [keys] config of every action should deserialize");
        let unknown = unknown_config_key_diagnostics(ignored, None);

        assert!(
            unknown.is_empty(),
            "keybind actions rejected by KeysConfigOverlay \
             (add them to the overlay struct + apply_field! + local_profile): {unknown:?}"
        );
    }

    #[test]
    fn config_diagnostic_summary_keeps_mixed_diagnostics_generic() {
        let diagnostics = vec![
            "invalid ui config: invalid type: string; keeping current ui settings".to_string(),
            "unknown config key keys.new_tabb; ignoring key".to_string(),
        ];

        assert_eq!(
            config_diagnostic_summary(&diagnostics).as_deref(),
            Some("config.toml; herdr config check")
        );
    }

    #[test]
    fn config_diagnostic_summary_reports_default_fallback() {
        let diagnostics = vec![
            "config parse error: TOML parse error at line 33, column 8\n   |\n33 | type = \"popup\"\n   |        ^^^^^^^\nunknown variant `popup`; using defaults"
                .to_string(),
        ];

        assert_eq!(
            config_diagnostic_summary(&diagnostics).as_deref(),
            Some("config.toml invalid; using defaults; herdr config check")
        );
    }

    #[test]
    fn config_diagnostic_summary_reports_unreadable_config_impact() {
        let startup = vec!["config read error: permission denied; using defaults".to_string()];
        assert_eq!(
            config_diagnostic_summary(&startup).as_deref(),
            Some("config.toml unreadable; using defaults; herdr config check")
        );

        let reload =
            vec!["config read error: permission denied; keeping current config".to_string()];
        assert_eq!(
            config_diagnostic_summary(&reload).as_deref(),
            Some("config.toml unreadable; keeping current config; herdr config check")
        );
    }

    #[test]
    fn config_diagnostic_summary_reports_retained_live_config() {
        let diagnostics = vec![
            "config parse error: TOML parse error at line 7, column 4; keeping current config"
                .to_string(),
        ];

        assert_eq!(
            config_diagnostic_summary(&diagnostics).as_deref(),
            Some("config.toml invalid; keeping current config; herdr config check")
        );
    }

    #[test]
    fn config_loaders_report_unreadable_path() {
        let _guard = crate::config::test_config_env_lock().lock().unwrap();
        let path =
            std::env::temp_dir().join(format!("herdr-config-unreadable-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        std::env::set_var(CONFIG_PATH_ENV_VAR, &path);

        let startup = Config::load();
        assert!(startup
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("config read error")
                && diagnostic.contains("using defaults")));

        let reload = load_live_config().unwrap_err();
        assert!(reload.iter().any(|diagnostic| {
            diagnostic.contains("config read error")
                && diagnostic.contains("keeping current config")
        }));

        std::env::remove_var(CONFIG_PATH_ENV_VAR);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn load_live_config_parses_session_section() {
        let loaded = load_live_config_from_str(
            r#"
[session]
resume_agents_on_restore = true
"#,
        )
        .unwrap();

        assert!(loaded.config.session.resume_agents_on_restore);
        assert!(loaded.diagnostics.is_empty());
        assert!(loaded.invalid_sections.is_empty());
    }

    #[test]
    fn agents_is_a_known_top_level_section() {
        let (keys, diagnostics) = unknown_top_level_sections_from_str(
            r#"
[agents.codex]
app_server = true
"#,
        );

        assert!(keys.is_empty(), "{keys:?}");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn load_live_config_keeps_the_codex_agent_section() {
        let loaded = load_live_config_from_str(
            r#"
[agents.codex]
app_server = true
name_threads = true
"#,
        )
        .unwrap();

        assert!(loaded.config.agents.codex.app_server);
        assert!(loaded.config.agents.codex.name_threads);
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
        assert!(loaded.invalid_sections.is_empty());
    }

    #[test]
    fn load_live_config_warns_about_unknown_theme_names() {
        let loaded = load_live_config_from_str(
            r#"
[theme]
name = "catppucin"
"#,
        )
        .unwrap();

        assert_eq!(loaded.diagnostics.len(), 1);
        assert!(loaded.diagnostics[0].contains("theme.name = \"catppucin\""));
    }

    #[test]
    fn load_live_config_warns_about_unknown_top_level_sections() {
        let loaded = load_live_config_from_str(
            r#"
[toast]
delivery = "system"

[ui.toast]
delivery = "herdr"
"#,
        )
        .unwrap();

        assert_eq!(
            loaded.diagnostics,
            vec!["unknown config section [toast]; did you mean [ui.toast]? ignoring section"]
        );
        assert!(loaded.invalid_sections.is_empty());
        assert_eq!(
            loaded.config.ui.toast.delivery,
            super::super::ToastDelivery::Herdr
        );
    }

    #[test]
    fn load_live_config_warns_about_unknown_keys_and_applies_known_siblings() {
        let loaded = load_live_config_from_str(
            r##"
plugin = []

[theme.custom]
accentt = "#ffffff"

[advanced]
scrollback_lines = 42

[keys]
fullscreen = "prefix+z"
new_tabb = "prefix+t"

[[keys.command]]
key = "prefix+g"
command = "git status"
descrption = "status"

[ui]
mouse_capture = false
mouse_captur = true
"foo.bar" = true
"foo.?.bar" = false

[ui.toast]
enabled = true
delivry = "system"

[ui.sidebar.agents.rows_by_agent]
claude = [["terminal_title"]]
"##,
        )
        .unwrap();

        assert_eq!(
            loaded.diagnostics,
            vec![
                "unknown config key plugin; ignoring key",
                "unknown config key theme.custom.accentt; ignoring key",
                "unknown config key keys.command.0.descrption; ignoring key",
                "unknown config key keys.new_tabb; ignoring key",
                "unknown config key ui.\"foo.?.bar\"; ignoring key",
                "unknown config key ui.\"foo.bar\"; ignoring key",
                "unknown config key ui.mouse_captur; ignoring key",
                "unknown config key ui.toast.delivry; ignoring key",
            ]
        );
        assert!(loaded.invalid_sections.is_empty());
        assert_eq!(loaded.config.advanced.scrollback_limit_bytes, 42);
        assert!(!loaded.config.ui.mouse_capture);
        assert_eq!(
            loaded.config.ui.toast.delivery,
            super::super::ToastDelivery::Herdr
        );
        assert!(loaded
            .config
            .keybinds()
            .zoom
            .bindings
            .iter()
            .any(|binding| binding.label == "prefix+z"));
    }

    #[test]
    fn load_live_config_accepts_legacy_agent_panel_scope_without_warning() {
        let loaded = load_live_config_from_str(
            r#"
[ui]
agent_panel_scope = "current"
agent_panel_sort = "priority"
"#,
        )
        .unwrap();

        assert!(loaded.diagnostics.is_empty());
        assert!(loaded.invalid_sections.is_empty());
        assert_eq!(
            loaded.config.ui.agent_panel_sort,
            super::super::AgentPanelSortConfig::Priority
        );
    }

    #[test]
    fn load_live_config_reports_a_bad_value_and_an_unknown_key_separately() {
        let loaded = load_live_config_from_str(
            r#"
[ui]
mouse_capture = "yes"
mouse_captur = true
"#,
        )
        .unwrap();

        // The bad value is one warning with its line; the unknown key next to
        // it is another; the section is not dropped (fork issue 133).
        assert_eq!(loaded.diagnostics.len(), 2, "{:?}", loaded.diagnostics);
        assert!(loaded.diagnostics[0].contains("invalid value for ui.mouse_capture at line 3"));
        assert!(loaded.diagnostics[1].starts_with("unknown config key ui.mouse_captur"));
        assert!(loaded.invalid_sections.is_empty());
    }

    #[test]
    fn startup_config_accepts_legacy_agent_panel_scope_without_warning() {
        let _guard = crate::config::test_config_env_lock().lock().unwrap();
        let path = std::env::temp_dir().join(format!(
            "herdr-config-legacy-agent-panel-scope-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "[ui]\nagent_panel_scope = \"all\"\n").unwrap();
        std::env::set_var(CONFIG_PATH_ENV_VAR, &path);

        let loaded = Config::load();

        std::env::remove_var(CONFIG_PATH_ENV_VAR);
        let _ = std::fs::remove_file(path);

        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
    }

    #[test]
    fn startup_config_load_warns_about_unknown_top_level_sections() {
        let _guard = crate::config::test_config_env_lock().lock().unwrap();
        let path = std::env::temp_dir().join(format!(
            "herdr-config-unknown-section-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"
[[plugin]]
id = "example"

[ui.toast]
delivery = "system"
"#,
        )
        .unwrap();
        std::env::set_var(CONFIG_PATH_ENV_VAR, &path);

        let loaded = Config::load();

        assert_eq!(
            loaded.diagnostics,
            vec!["unknown config section [[plugin]]; ignoring section"]
        );
        assert_eq!(
            loaded.config.ui.toast.delivery,
            super::super::ToastDelivery::System
        );

        std::env::remove_var(CONFIG_PATH_ENV_VAR);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn remove_keybinding_config_sections_removes_keys_tables_only() {
        let content = r#"onboarding = false

[theme]
name = "catppuccin"

[keys]
prefix = "ctrl+a"
new_tab = "c"

[[keys.command]]
key = "g"
command = "lazygit"

[keys.indexed]
tabs = "ctrl"

[ui]
mouse_capture = false
"#;

        let (updated, removed) = remove_keybinding_config_sections(content);

        assert!(removed);
        assert!(updated.contains("onboarding = false"));
        assert!(updated.contains("[theme]\nname = \"catppuccin\""));
        assert!(updated.contains("[ui]\nmouse_capture = false"));
        assert!(!updated.contains("[keys]"));
        assert!(!updated.contains("[[keys.command]]"));
        assert!(!updated.contains("[keys.indexed]"));
        assert!(toml::from_str::<toml::Value>(&updated).is_ok());
    }

    #[test]
    fn remove_keybinding_config_sections_reports_noop_without_keys() {
        let content = "[ui]\nmouse_capture = true\n";
        let (updated, removed) = remove_keybinding_config_sections(content);
        assert!(!removed);
        assert_eq!(updated, content);
    }

    #[test]
    fn normalize_utf8_bom_removes_a_leading_bom() {
        let content = "\u{feff}onboarding = false\n[terminal]\n";
        assert_eq!(
            normalize_utf8_bom(content),
            "onboarding = false\n[terminal]\n"
        );
    }

    #[test]
    fn normalize_utf8_bom_recovers_from_a_displaced_mid_file_bom() {
        let content = "onboarding = false\n\u{feff}[terminal]\ndefault_shell = \"pwsh.exe\"\n";
        let normalized = normalize_utf8_bom(content);
        assert_eq!(
            normalized,
            "onboarding = false\n[terminal]\ndefault_shell = \"pwsh.exe\"\n"
        );
        assert!(normalized.parse::<toml::Value>().is_ok());
    }

    #[test]
    fn normalize_utf8_bom_preserves_boms_in_multiline_basic_strings() {
        let content = "[theme]\nname = \"\"\"\nfirst\n\u{feff}second\n\"\"\"\n";
        assert!(content.parse::<toml::Value>().is_ok());
        assert_eq!(normalize_utf8_bom(content), content);
    }

    #[test]
    fn normalize_utf8_bom_preserves_boms_in_multiline_literal_strings() {
        let content = "[theme]\nname = '''\nfirst\n\u{feff}second\n'''\n";
        assert!(content.parse::<toml::Value>().is_ok());
        assert_eq!(normalize_utf8_bom(content), content);
    }

    #[test]
    fn normalize_utf8_bom_preserves_string_boms_despite_other_errors() {
        let content = "[theme]\nname = \"\"\"\nfirst\n\u{feff}second\n\"\"\"\nbroken = \n";
        assert!(content.parse::<toml::Value>().is_err());
        assert_eq!(normalize_utf8_bom(content), content);
    }

    #[test]
    fn config_load_recovers_from_a_mid_file_bom() {
        let _guard = crate::config::test_config_env_lock().lock().unwrap();
        let path = std::env::temp_dir().join(format!(
            "herdr-config-mid-file-bom-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            b"onboarding = false\n\xEF\xBB\xBF[terminal]\ndefault_shell = \"pwsh.exe\"\n",
        )
        .unwrap();
        std::env::set_var(CONFIG_PATH_ENV_VAR, &path);

        let loaded = Config::load();

        std::env::remove_var(CONFIG_PATH_ENV_VAR);
        let _ = std::fs::remove_file(path);

        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
        assert_eq!(loaded.config.terminal.default_shell, "pwsh.exe");
    }

    /// The beta-133 italic case (fork issue 162): a sidebar token style with a
    /// key this build does not know. It used to fail the whole parse, because
    /// `RawSidebarToken` is untagged and the error lost the key.
    const UNKNOWN_STYLE_KEY_CONFIG: &str = r##"
[theme.custom]
accent = "#112233"

[ui]
mouse_capture = false

[ui.sidebar.spaces]
rows = [
  ["state_icon", "workspace"],
  [{ token = "branch", dim = false }, "git_status", { token = "$asks", fg = "#FFD60A", bold = true, wobble = true }],
]
"##;

    fn startup_load(content: &str, tag: &str) -> LoadedConfig {
        let _guard = crate::config::test_config_env_lock().lock().unwrap();
        let path =
            std::env::temp_dir().join(format!("herdr-config-{tag}-{}.toml", std::process::id()));
        std::fs::write(&path, content).unwrap();
        std::env::set_var(CONFIG_PATH_ENV_VAR, &path);
        let loaded = Config::load();
        std::env::remove_var(CONFIG_PATH_ENV_VAR);
        let _ = std::fs::remove_file(path);
        loaded
    }

    fn assert_unknown_style_key_is_forgiven(loaded: &LoadedConfig) {
        assert_eq!(
            loaded.diagnostics.len(),
            1,
            "exactly one diagnostic: {:?}",
            loaded.diagnostics
        );
        let diagnostic = &loaded.diagnostics[0];
        assert!(diagnostic.contains("wobble"), "names the key: {diagnostic}");
        assert!(
            diagnostic.contains("ui.sidebar.spaces.rows"),
            "names the row: {diagnostic}"
        );
        // The rest of the config applies.
        assert!(!loaded.config.ui.mouse_capture);
        assert!(loaded.config.theme.custom.is_some());
        // Both rows survive, and the token keeps its known fields.
        let rows = &loaded.config.ui.sidebar.spaces.rows;
        assert_eq!(rows.len(), 2, "{rows:?}");
        let (token, style) = rows[1][2].parts();
        assert_eq!(
            token,
            &crate::config::SpaceSidebarToken::Custom("asks".into())
        );
        assert_eq!(style.bold, Some(true));
        assert!(style.fg.is_some());
    }

    #[test]
    fn startup_forgives_an_unknown_sidebar_token_style_key() {
        let loaded = startup_load(UNKNOWN_STYLE_KEY_CONFIG, "unknown-style-key");
        assert_unknown_style_key_is_forgiven(&loaded);
    }

    #[test]
    fn reload_forgives_an_unknown_sidebar_token_style_key() {
        let loaded = load_live_config_from_str(UNKNOWN_STYLE_KEY_CONFIG).unwrap();
        assert_unknown_style_key_is_forgiven(&loaded);
        assert!(loaded.invalid_sections.is_empty());
    }

    #[test]
    fn a_wrong_value_type_in_a_style_key_drops_only_that_value_with_a_diagnostic() {
        // A multi-line rows array, so a one-line value repair could not blank
        // it: the bad `bold` is dropped and the rest of the file applies, at
        // startup as on reload (fork issue 163).
        let content = r##"
[ui]
mouse_capture = false

[ui.sidebar.spaces]
rows = [
  ["workspace"],
  [
    { token = "$asks", bold = "yes", italic = true, fg = 5, keep = 1, dim = [1] },
    { token = "branch", truncate = 7 },
  ],
]

[ui.sidebar.agents]
rows = [
  ["state_icon", { token = "agent", bold = "nope", keep = true }],
]
"##;
        let startup = startup_load(content, "bad-style-type");
        let reload = load_live_config_from_str(content).unwrap();
        for loaded in [&startup, &reload] {
            assert!(!loaded.config.ui.mouse_capture, "the rest of [ui] applied");
            // The other rows survive.
            assert_eq!(loaded.config.ui.sidebar.spaces.rows.len(), 2);
            assert_eq!(loaded.config.ui.sidebar.spaces.rows[0].len(), 1);
            // The valid keys of the bad token survive; the bad ones are unset.
            let (_, style) = loaded.config.ui.sidebar.spaces.rows[1][0].parts();
            assert_eq!(style.italic, Some(true));
            assert_eq!(
                (style.bold, style.fg, style.keep, style.dim),
                (None, None, None, None)
            );
            let (_, agent) = loaded.config.ui.sidebar.agents.rows[0][1].parts();
            assert_eq!((agent.bold, agent.keep), (None, Some(true)));
        }
        // `Config::load` reports through `collect_diagnostics`; the reload path
        // reports through the same `sidebar_style_diagnostics` in the app.
        let diagnostics = crate::config::sidebar_style_diagnostics(&startup.config.ui.sidebar);
        assert_eq!(
            diagnostics,
            vec![
                "ui.sidebar.agents.rows[0][1] has an invalid `bold` (expected true or false); ignoring it",
                "ui.sidebar.spaces.rows[1][0] has an invalid `fg` (expected a #RGB or #RRGGBB colour); ignoring it",
                "ui.sidebar.spaces.rows[1][0] has an invalid `bold` (expected true or false); ignoring it",
                "ui.sidebar.spaces.rows[1][0] has an invalid `dim` (expected true or false); ignoring it",
                "ui.sidebar.spaces.rows[1][0] has an invalid `keep` (expected true or false); ignoring it",
                "ui.sidebar.spaces.rows[1][1] has an invalid `truncate` (expected \"start\" or \"end\"); ignoring it",
            ]
        );
        for diagnostic in &diagnostics {
            assert!(
                startup.diagnostics.contains(diagnostic),
                "startup reports it: {diagnostic}"
            );
        }
    }

    #[test]
    fn a_valid_style_file_has_no_diagnostics_and_no_invalid_keys() {
        let content = r##"
[ui]
mouse_capture = false

[ui.sidebar.spaces]
rows = [
  ["workspace", { token = "$asks", fg = "#ff0000", bold = true, dim = false, italic = true, keep = true, truncate = "start" }],
]
"##;
        let startup = startup_load(content, "valid-style");
        assert!(startup.diagnostics.is_empty(), "{:?}", startup.diagnostics);
        let (_, style) = startup.config.ui.sidebar.spaces.rows[0][1].parts();
        assert_eq!(style.invalid, 0);
        assert_eq!(style.bold, Some(true));
        assert_eq!(
            style.truncate,
            Some(crate::config::SidebarTokenTruncate::Start)
        );
    }

    #[test]
    fn an_unrecognized_truncate_value_is_forgiven_on_startup_and_reload() {
        let content = r##"
[ui]
mouse_capture = false

[ui.sidebar.spaces]
rows = [
  ["workspace"],
  [{ token = "$asks", keep = true, truncate = "middle" }],
]
"##;
        let startup = startup_load(content, "truncate-unrecognized");
        let reload = load_live_config_from_str(content).unwrap();
        assert_eq!(
            startup.diagnostics,
            vec![
                "ui.sidebar.spaces.rows[1][0] has an unrecognized truncate value (expected \"start\" or \"end\"); using \"end\""
            ]
        );
        // The reload path reports it through the app (`sidebar_style_diagnostics`
        // in `apply_live_config`); here the config itself carries the value.
        assert!(!startup.config.ui.mouse_capture && !reload.config.ui.mouse_capture);
        for loaded in [&startup, &reload] {
            let (_, style) = loaded.config.ui.sidebar.spaces.rows[1][0].parts();
            assert_eq!(style.keep, Some(true));
        }
    }

    #[test]
    fn an_unknown_tab_bar_right_key_is_forgiven_with_one_diagnostic() {
        let content = r##"
[ui]
mouse_capture = false
tab_bar_right = [
  { type = "text", text = "hi", wobble = 1 },
  { type = "zoom" },
]
"##;
        for loaded in [
            startup_load(content, "tab-bar-unknown"),
            load_live_config_from_str(content).unwrap(),
        ] {
            assert_eq!(
                loaded.diagnostics,
                vec!["unknown config key ui.tab_bar_right.0.wobble; ignoring key"]
            );
            assert!(!loaded.config.ui.mouse_capture);
            assert_eq!(loaded.config.ui.tab_bar_right.len(), 2);
        }
    }

    #[test]
    fn a_bad_tab_bar_right_entry_is_hidden_with_a_diagnostic_and_the_rest_applies() {
        // Multi-line, with every kind of bad entry between good ones.
        let content = r##"
[ui]
mouse_capture = false
tab_bar_right = [
  { type = "zoom" },
  { type = "zoom", text = "x" },
  { type = "datetime", format = 5 },
  { type = "command", command = "status.sh", interval_seconds = "often" },
  { type = "text" },
  { type = "wobble" },
  { type = 3 },
  { text = "no type" },
  { type = "hostname" },
]
"##;
        for loaded in [
            startup_load(content, "tab-bar-bad-entries"),
            load_live_config_from_str(content).unwrap(),
        ] {
            assert!(!loaded.config.ui.mouse_capture, "the rest of [ui] applied");
            let entries = &loaded.config.ui.tab_bar_right;
            assert_eq!(entries.len(), 9);
            assert!(matches!(
                entries[0],
                crate::config::TabBarRightEntryConfig::Zoom
            ));
            assert!(matches!(
                entries[8],
                crate::config::TabBarRightEntryConfig::Hostname
            ));
            assert!(entries[1..8].iter().all(|entry| matches!(
                entry,
                crate::config::TabBarRightEntryConfig::Invalid { .. }
            )));
            assert_eq!(
                crate::config::tab_bar_right_diagnostics(entries),
                vec![
                    "ui.tab_bar_right[1] has `text`, which does not belong to type `zoom`; hiding entry",
                    "ui.tab_bar_right[2] has a wrong value type for `format` (expected a string); hiding entry",
                    "ui.tab_bar_right[3] has a wrong value type for `interval_seconds` (expected a whole number); hiding entry",
                    "ui.tab_bar_right[4] is missing `text`; hiding entry",
                    "ui.tab_bar_right[5] has an unknown type `wobble` (expected zoom, hostname, datetime, text or command); hiding entry",
                    "ui.tab_bar_right[6] has a wrong value type for `type` (expected a string); hiding entry",
                    "ui.tab_bar_right[7] is missing `type`; hiding entry",
                ]
            );
        }
        let startup = startup_load(content, "tab-bar-bad-entries-diag");
        assert!(startup
            .diagnostics
            .iter()
            .any(|d| d.contains("ui.tab_bar_right[2] has a wrong value type for `format`")));
    }
}
