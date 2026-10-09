use crossterm::event::{KeyCode, KeyModifiers};

mod io;
mod keybinds;
mod model;
mod sidebar;
mod sound;
mod tab_bar;
mod theme;
mod window_title;
mod write;

pub(crate) use self::model::HerdrToastConfig;

pub use self::{
    io::{
        config_diagnostic_summary, config_dir, config_path, load_live_config,
        remove_keybinding_config_sections, remove_section_key, state_dir, upsert_section_bool,
        upsert_section_value,
    },
    keybinds::{
        format_prefix_combos, jump_symbol, normalize_key_combo, terminal_key_matches_combo,
        ActionKeybinds, BindingConfig, CommandKeybindConfig, CustomCommandAction,
        CustomCommandKeybind, IndexedKeybind, KeyCombo, Keybinds, LiveKeybindConfig,
    },
    model::{
        clamp_display_panes_ms, validated_sidebar_bounds, AgentPanelSortConfig, CodexAgentConfig,
        Config, ConfigReloadReport, ConfigReloadStatus, HostCursorModeConfig, NewTerminalCwdConfig,
        NotificationCenterPositionConfig, PaneBorderActiveStyleConfig, PaneBordersConfig,
        ShellModeConfig, SidebarActiveBorderConfig, SidebarCollapsedModeConfig, SidebarFogStyle,
        SidebarOverflowConfig, SidebarStyleConfig, SortMotionConfig, SortMotionEasingConfig,
        StateSymbolsConfig, StatusIndicatorStyle, StatusSpinnerConfig, TabBarPositionConfig,
        ToastClipboardPosition, ToastConfig, ToastDelivery, ToastHerdrPosition, ToastHerdrSize,
        ToastPaneFeedback, UpdateChannelConfig, WorkspaceSortConfig, MAX_DISPLAY_PANES_MS,
        MAX_STATUS_SPINNER_MS, MAX_TOAST_DELAY_SECONDS, MIN_DISPLAY_PANES_MS,
        MIN_STATUS_SPINNER_MS,
    },
    sidebar::{
        AgentSidebarToken, AgentsSidebarConfig, SidebarConfig, SidebarTokenStyle,
        SpaceSidebarToken, SpacesSidebarConfig,
    },
    sound::SoundConfig,
    tab_bar::TabBarRightEntryConfig,
    theme::{parse_color, CustomThemeColors, ModeThemeColors, ThemeConfig, THEME_NAMES},
    window_title::{WindowTitlePart, WindowTitleTemplate, WindowTitleToken},
};

pub(crate) use self::keybinds::parse_key_combo;
#[cfg(test)]
pub(crate) use self::sidebar::SidebarTokenTruncate;
pub(crate) use self::write::{update_file_at, write_edit, ConfigEdit};

/// The fog colours this many rows next to a hidden edge.
pub(crate) const SIDEBAR_FOG_ROWS: usize = 2;
pub(crate) const SIDEBAR_FOG_MAX_PERCENT: i64 = 60;
pub(crate) const SIDEBAR_FOG_TINT_MAX: i64 = 100;
pub(crate) const SIDEBAR_FADE_MAX_PERCENT: i64 = 95;
pub(crate) const INACTIVE_PANE_DIM_MAX_PERCENT: i64 = 90;
pub(crate) use self::{
    io::upsert_top_level_bool,
    sidebar::sidebar_style_diagnostics,
    tab_bar::{
        parse_tab_bar_datetime_format, tab_bar_right_diagnostics,
        MAX_TAB_BAR_COMMAND_INTERVAL_SECONDS, MAX_TAB_BAR_COMMAND_TIMEOUT_SECONDS,
        MAX_TAB_BAR_RIGHT_ENTRIES,
    },
    theme::canonical_theme_name,
    window_title::{sanitize_window_title_text, window_title_diagnostics},
};

pub(crate) use self::{keybinds::CommandKeybindType, model::KeysConfig};

pub const CONFIG_PATH_ENV_VAR: &str = "HERDR_CONFIG_PATH";

pub(crate) fn is_keybinding_config_diagnostic(diagnostic: &str) -> bool {
    if diagnostic.starts_with("config parse error:") || diagnostic.starts_with("config read error:")
    {
        return false;
    }
    diagnostic.contains("keybinding") || diagnostic.contains("keys.")
}

pub(crate) fn config_diagnostic_summary_without_keybindings(
    diagnostics: &[String],
) -> Option<String> {
    let diagnostics = diagnostics
        .iter()
        .filter(|diagnostic| !is_keybinding_config_diagnostic(diagnostic))
        .cloned()
        .collect::<Vec<_>>();
    config_diagnostic_summary(&diagnostics)
}
pub const DEFAULT_SCROLLBACK_LIMIT_BYTES: usize = 10_000_000;
pub const DEFAULT_MOUSE_SCROLL_LINES: usize = 3;
pub const DEFAULT_MOBILE_WIDTH_THRESHOLD: u16 = 64;
pub const DEFAULT_HEADLESS_COLS: u16 = 120;
pub const DEFAULT_HEADLESS_ROWS: u16 = 40;

#[cfg(test)]
pub(crate) fn app_dir_name() -> &'static str {
    io::app_dir_name()
}

#[cfg(test)]
pub(crate) fn test_config_env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

impl Config {
    pub fn should_show_onboarding(&self) -> bool {
        self.onboarding.unwrap_or(true)
    }

    pub fn kitty_graphics_enabled(&self) -> bool {
        self.terminal
            .kitty_graphics
            .or(self.experimental.kitty_graphics)
            .unwrap_or(true)
    }

    pub fn prefix_keys(&self) -> Vec<(KeyCode, KeyModifiers)> {
        self.validated_keybinds().1
    }

    /// Parsed keybinds for Herdr actions.
    pub fn keybinds(&self) -> Keybinds {
        self.validated_keybinds().3
    }

    pub fn collect_diagnostics(&self) -> Vec<String> {
        let (prefix_diag, _, keybind_diags, _) = self.validated_keybinds();
        prefix_diag
            .into_iter()
            .chain(keybind_diags)
            .chain(self.remote_image_paste_key().err())
            .chain(self.theme.diagnostics())
            .chain(self.ui.sound.diagnostics())
            .chain(self.ui.state_symbols.diagnostics())
            .chain(tab_bar_right_diagnostics(&self.ui.tab_bar_right))
            .chain(sidebar_style_diagnostics(&self.ui.sidebar))
            .chain(window_title_diagnostics(&self.ui.window_title))
            .chain(self.invalid_sidebar_bounds_diagnostic())
            .chain(self.invalid_headless_size_diagnostic())
            .chain(self.display_panes_diagnostic())
            .chain(self.sidebar_fog_diagnostics())
            .chain(self.inactive_pane_dim_diagnostics())
            .collect()
    }

    pub(crate) fn headless_size(&self) -> (u16, u16) {
        if self.invalid_headless_size_diagnostic().is_some() {
            (DEFAULT_HEADLESS_COLS, DEFAULT_HEADLESS_ROWS)
        } else {
            (self.server.headless_cols, self.server.headless_rows)
        }
    }

    pub(crate) fn invalid_headless_size_diagnostic(&self) -> Option<String> {
        (self.server.headless_cols == 0 || self.server.headless_rows == 0).then(|| {
            format!(
                "server.headless_cols and server.headless_rows must be greater than zero (got {}x{})",
                self.server.headless_cols, self.server.headless_rows
            )
        })
    }

    /// The fog strengths in effect: the percent for the nearest and the next
    /// row (a missing entry is 0, no fog), and the tint share. Out-of-range
    /// values are clamped; [`Self::sidebar_fog_diagnostics`] says so.
    pub(crate) fn sidebar_fog(&self) -> ([u32; SIDEBAR_FOG_ROWS], u32) {
        let mut percent = [0u32; SIDEBAR_FOG_ROWS];
        for (slot, value) in percent.iter_mut().zip(&self.ui.sidebar_fog) {
            *slot = (*value).clamp(0, SIDEBAR_FOG_MAX_PERCENT) as u32;
        }
        (
            percent,
            self.ui.sidebar_fog_tint.clamp(0, SIDEBAR_FOG_TINT_MAX) as u32,
        )
    }

    /// The text fade per fogged row, in percent toward the fog base (a missing
    /// entry is 0, no fade). Out-of-range values are clamped;
    /// [`Self::sidebar_fog_diagnostics`] says so.
    pub(crate) fn sidebar_fade(&self) -> [u32; SIDEBAR_FOG_ROWS] {
        let mut percent = [0u32; SIDEBAR_FOG_ROWS];
        for (slot, value) in percent.iter_mut().zip(&self.ui.sidebar_fade) {
            *slot = (*value).clamp(0, SIDEBAR_FADE_MAX_PERCENT) as u32;
        }
        percent
    }

    /// The unfocused-pane text fade in percent, clamped; see
    /// [`Self::inactive_pane_dim_diagnostics`].
    pub(crate) fn inactive_pane_dim(&self) -> u32 {
        self.ui
            .inactive_pane_dim
            .clamp(0, INACTIVE_PANE_DIM_MAX_PERCENT) as u32
    }

    pub(crate) fn inactive_pane_dim_diagnostics(&self) -> Option<String> {
        let value = self.ui.inactive_pane_dim;
        (!(0..=INACTIVE_PANE_DIM_MAX_PERCENT).contains(&value)).then(|| {
            format!(
                "ui.inactive_pane_dim ({value}) is outside 0..={INACTIVE_PANE_DIM_MAX_PERCENT}; using {}",
                value.clamp(0, INACTIVE_PANE_DIM_MAX_PERCENT)
            )
        })
    }

    /// The fog style in effect; an unknown value is "lift" and
    /// [`Self::sidebar_fog_diagnostics`] says so.
    pub(crate) fn sidebar_fog_style(&self) -> SidebarFogStyle {
        SidebarFogStyle::parse(&self.ui.sidebar_fog_style).unwrap_or_default()
    }

    pub(crate) fn sidebar_fog_diagnostics(&self) -> Vec<String> {
        let mut out = Vec::new();
        if SidebarFogStyle::parse(&self.ui.sidebar_fog_style).is_none() {
            out.push(format!(
                "ui.sidebar_fog_style (\"{}\") is not lift, dim or both; using lift",
                self.ui.sidebar_fog_style
            ));
        }
        if self.ui.sidebar_fog.len() > SIDEBAR_FOG_ROWS {
            out.push(format!(
                "ui.sidebar_fog has {} entries; only the first {SIDEBAR_FOG_ROWS} are used",
                self.ui.sidebar_fog.len()
            ));
        }
        for (index, value) in self
            .ui
            .sidebar_fog
            .iter()
            .enumerate()
            .take(SIDEBAR_FOG_ROWS)
        {
            if !(0..=SIDEBAR_FOG_MAX_PERCENT).contains(value) {
                out.push(format!(
                    "ui.sidebar_fog[{index}] ({value}) is outside 0..={SIDEBAR_FOG_MAX_PERCENT}; using {}",
                    (*value).clamp(0, SIDEBAR_FOG_MAX_PERCENT)
                ));
            }
        }
        if self.ui.sidebar_fade.len() > SIDEBAR_FOG_ROWS {
            out.push(format!(
                "ui.sidebar_fade has {} entries; only the first {SIDEBAR_FOG_ROWS} are used",
                self.ui.sidebar_fade.len()
            ));
        }
        for (index, value) in self
            .ui
            .sidebar_fade
            .iter()
            .enumerate()
            .take(SIDEBAR_FOG_ROWS)
        {
            if !(0..=SIDEBAR_FADE_MAX_PERCENT).contains(value) {
                out.push(format!(
                    "ui.sidebar_fade[{index}] ({value}) is outside 0..={SIDEBAR_FADE_MAX_PERCENT}; using {}",
                    (*value).clamp(0, SIDEBAR_FADE_MAX_PERCENT)
                ));
            }
        }
        let tint = self.ui.sidebar_fog_tint;
        if !(0..=SIDEBAR_FOG_TINT_MAX).contains(&tint) {
            out.push(format!(
                "ui.sidebar_fog_tint ({tint}) is outside 0..={SIDEBAR_FOG_TINT_MAX}; using {}",
                tint.clamp(0, SIDEBAR_FOG_TINT_MAX)
            ));
        }
        out
    }

    fn display_panes_diagnostic(&self) -> Option<String> {
        let ms = self.ui.display_panes_ms;
        (clamp_display_panes_ms(ms) != ms).then(|| {
            format!(
                "ui.display_panes_ms ({ms}) is outside {MIN_DISPLAY_PANES_MS}..={MAX_DISPLAY_PANES_MS}; using {}",
                clamp_display_panes_ms(ms)
            )
        })
    }

    pub(crate) fn invalid_sidebar_bounds_diagnostic(&self) -> Option<String> {
        validated_sidebar_bounds(self.ui.sidebar_min_width, self.ui.sidebar_max_width)
            .is_none()
            .then(|| {
                format!(
                    "ui.sidebar_min_width ({}) is greater than sidebar_max_width ({})",
                    self.ui.sidebar_min_width, self.ui.sidebar_max_width
                )
            })
    }

    pub(crate) fn remote_image_paste_key(&self) -> Result<Option<(KeyCode, KeyModifiers)>, String> {
        let raw = self.keys.remote_image_paste.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        parse_key_combo(raw).map(Some).ok_or_else(|| {
            format!("invalid keybinding: keys.remote_image_paste = {raw:?}; disabling binding")
        })
    }

    pub(crate) fn live_keybinds_with_diagnostics(
        &self,
    ) -> Result<(LiveKeybindConfig, Vec<String>), Vec<String>> {
        let (prefix_diag, prefix, keybind_diags, keybinds) = self.validated_keybinds();
        if let Some(prefix_diag) = prefix_diag {
            Err(std::iter::once(prefix_diag).chain(keybind_diags).collect())
        } else {
            Ok((LiveKeybindConfig { prefix, keybinds }, keybind_diags))
        }
    }

    pub(crate) fn local_keybindings_profile_toml(&self) -> Result<String, toml::ser::Error> {
        #[derive(serde::Serialize)]
        struct KeysProfile {
            keys: model::KeysConfigOverlay,
        }

        let mut keys = self.keys.local_profile(&self.keybinds());
        keys.set_prefixes(&self.prefix_keys());
        toml::to_string_pretty(&KeysProfile { keys })
    }
}

pub(crate) fn keybindings_from_profile_toml(profile: &str) -> Result<LiveKeybindConfig, String> {
    let config = toml::from_str::<Config>(profile)
        .map_err(|err| format!("invalid keybinding profile: {err}"))?;
    config
        .live_keybinds_with_diagnostics()
        .map(|(keybinds, _diagnostics)| keybinds)
        .map_err(|diagnostics| diagnostics.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_keybindings_profile_preserves_plus_prefixes() {
        let config: Config = toml::from_str(
            "[keys]\nprefix = [\"ctrl+plus\", \"alt+plus\"]\nnew_tab = \"prefix+plus\"\n",
        )
        .unwrap();
        let profile = config.local_keybindings_profile_toml().unwrap();
        let restored = keybindings_from_profile_toml(&profile).unwrap();
        assert_eq!(restored.prefix, config.prefix_keys());
        assert_eq!(
            restored.keybinds.new_tab.bindings,
            config.keybinds().new_tab.bindings
        );
    }

    #[test]
    fn local_keybindings_profile_includes_defaults_and_excludes_commands() {
        let config: Config = toml::from_str(
            r#"
[keys]
prefix = "ctrl+a"
new_tab = "prefix+t"

[[keys.command]]
key = "prefix+g"
command = "lazygit"
"#,
        )
        .unwrap();

        let profile = config.local_keybindings_profile_toml().unwrap();
        assert!(profile.contains("[keys]"));
        assert!(profile.contains("prefix = \"ctrl+a\""));
        assert!(profile.contains("new_tab = \"prefix+t\""));
        assert!(profile.contains("next_tab = \"prefix+n\""));
        assert!(!profile.contains("lazygit"));
        assert!(!profile.contains("command ="));
        assert!(!profile.contains("[[keys.command]]"));
    }

    #[test]
    fn local_keybindings_profile_publishes_the_effective_prefix_fallback() {
        let config: Config = toml::from_str(
            r#"
[keys]
prefix = "ctrl+"
"#,
        )
        .unwrap();

        let profile = config.local_keybindings_profile_toml().unwrap();
        let keybinds = keybindings_from_profile_toml(&profile).unwrap();

        assert!(profile.contains("prefix = \"ctrl+b\""));
        assert_eq!(keybinds.prefix, config.prefix_keys());
    }

    #[test]
    fn local_keybindings_profile_publishes_additional_prefixes_for_old_clients() {
        let config: Config = toml::from_str(
            r#"
[keys]
prefix = ["ctrl+space", "ctrl+s"]
"#,
        )
        .unwrap();

        let profile = config.local_keybindings_profile_toml().unwrap();

        // Generation-1 clients parse `prefix` as a single string; the extra
        // prefixes ride in the optional `extra_prefixes` field they ignore.
        assert!(profile.contains("prefix = \"ctrl+space\""));
        assert!(!profile.contains("prefix = ["));
        assert!(profile.contains("extra_prefixes = [\"ctrl+s\"]"));

        let keybinds = keybindings_from_profile_toml(&profile).unwrap();
        assert_eq!(keybinds.prefix, config.prefix_keys());
    }

    #[test]
    fn local_keybindings_profile_omits_extra_prefixes_for_a_single_prefix() {
        let config: Config = toml::from_str(
            r#"
[keys]
prefix = "ctrl+b"
"#,
        )
        .unwrap();
        let profile = config.local_keybindings_profile_toml().unwrap();
        assert!(!profile.contains("extra_prefixes"));
    }

    #[test]
    fn local_keybindings_profile_preserves_user_default_provenance() {
        let config: Config = toml::from_str(
            r#"
[keys]
zoom = "prefix+?"
"#,
        )
        .unwrap();

        let profile = config.local_keybindings_profile_toml().unwrap();
        let round_tripped: Config = toml::from_str(&profile).unwrap();

        assert!(profile.contains("zoom = \"prefix+?\""));
        assert!(!profile.contains("help = \"prefix+?\""));
        assert!(round_tripped
            .keybinds()
            .zoom
            .bindings
            .iter()
            .any(|binding| binding.label == "prefix+?"));
        assert!(round_tripped.keybinds().help.bindings.is_empty());
    }

    #[test]
    fn local_keybindings_profile_omits_default_displaced_by_user_prefix() {
        let config: Config = toml::from_str(
            r#"
[keys]
prefix = "n"
"#,
        )
        .unwrap();

        let profile = config.local_keybindings_profile_toml().unwrap();
        let round_tripped: Config = toml::from_str(&profile).unwrap();

        assert!(profile.contains("prefix = \"n\""));
        assert!(!profile.contains("next_tab = \"prefix+n\""));
        assert!(round_tripped.keybinds().next_tab.bindings.is_empty());
    }

    #[test]
    fn local_keybindings_profile_preserves_legacy_indexed_tab_source() {
        let config: Config = toml::from_str(
            r#"
[keys.indexed]
tabs = "ctrl"
"#,
        )
        .unwrap();

        let profile = config.local_keybindings_profile_toml().unwrap();
        let round_tripped: Config = toml::from_str(&profile).unwrap();
        let keybinds = round_tripped.keybinds();
        let switch_tab_labels: Vec<_> = keybinds
            .switch_tab
            .iter()
            .map(|binding| binding.label.as_str())
            .collect();

        assert!(profile.contains("[keys.indexed]"));
        assert!(profile.contains("tabs = \"ctrl\""));
        assert!(!profile.contains("switch_tab = \"prefix+1..9\""));
        assert_eq!(switch_tab_labels.len(), 9);
        assert!(switch_tab_labels
            .iter()
            .all(|label| label.starts_with("ctrl+")));
    }

    #[test]
    fn local_keybindings_profile_keeps_invalid_legacy_indexed_default_disabled() {
        let config: Config = toml::from_str(
            r#"
[keys.indexed]
tabs = "bogus"
"#,
        )
        .unwrap();

        let profile = config.local_keybindings_profile_toml().unwrap();
        let round_tripped: Config = toml::from_str(&profile).unwrap();

        assert!(profile.contains("[keys.indexed]"));
        assert!(profile.contains("tabs = \"bogus\""));
        assert!(!profile.contains("switch_tab = \"prefix+1..9\""));
        assert!(round_tripped.keybinds().switch_tab.is_empty());
    }

    #[test]
    fn local_keybindings_profile_keeps_default_displaced_by_omitted_command_disabled() {
        let config: Config = toml::from_str(
            r#"
[[keys.command]]
key = "prefix+n"
command = "echo next"
"#,
        )
        .unwrap();

        let profile = config.local_keybindings_profile_toml().unwrap();
        let round_tripped: Config = toml::from_str(&profile).unwrap();

        assert!(!profile.contains("[[keys.command]]"));
        assert!(!profile.contains("command ="));
        assert!(profile.contains("next_tab = \"\""));
        assert!(round_tripped.keybinds().next_tab.bindings.is_empty());
    }

    #[test]
    fn local_keybindings_profile_preserves_partially_displaced_indexed_default() {
        let config: Config = toml::from_str(
            r#"
[[keys.command]]
key = "prefix+1"
command = "echo one"
"#,
        )
        .unwrap();

        let profile = config.local_keybindings_profile_toml().unwrap();
        let round_tripped: Config = toml::from_str(&profile).unwrap();
        let keybinds = round_tripped.keybinds();
        let switch_tab_labels: Vec<_> = keybinds
            .switch_tab
            .iter()
            .map(|binding| binding.label.as_str())
            .collect();

        assert!(!profile.contains("[[keys.command]]"));
        assert!(!profile.contains("switch_tab = \"prefix+1..9\""));
        assert!(profile.contains("\"prefix+2\""));
        assert!(profile.contains("\"prefix+9\""));
        assert!(!switch_tab_labels.contains(&"prefix+1"));
        assert_eq!(switch_tab_labels.len(), 8);
        assert!(switch_tab_labels
            .iter()
            .all(|label| label.starts_with("prefix+")));
    }

    #[test]
    fn remote_image_paste_key_defaults_to_ctrl_v() {
        let config = Config::default();
        assert_eq!(
            config.remote_image_paste_key().unwrap(),
            Some((KeyCode::Char('v'), KeyModifiers::CONTROL))
        );
    }

    #[test]
    fn remote_image_paste_key_can_be_disabled() {
        let config: Config = toml::from_str("[keys]\nremote_image_paste = ''\n").unwrap();
        assert_eq!(config.remote_image_paste_key().unwrap(), None);
    }

    #[test]
    fn ui_host_cursor_defaults_to_auto_and_parses_overrides() {
        let default_config = Config::default();
        assert_eq!(default_config.ui.host_cursor, HostCursorModeConfig::Auto);

        let native: Config = toml::from_str("[ui]\nhost_cursor = 'native'\n").unwrap();
        assert_eq!(native.ui.host_cursor, HostCursorModeConfig::Native);

        let drawn: Config = toml::from_str("[ui]\nhost_cursor = 'drawn'\n").unwrap();
        assert_eq!(drawn.ui.host_cursor, HostCursorModeConfig::Drawn);
    }
}
