use std::{collections::BTreeSet, num::NonZeroUsize};

use crossterm::event::KeyModifiers;
use serde::{de, Deserialize, Deserializer, Serialize};

use super::{
    ActionKeybinds, BindingConfig, CommandKeybindConfig, IndexedKeybind, Keybinds, SidebarConfig,
    SoundConfig, TabBarRightEntryConfig, ThemeConfig, DEFAULT_MOBILE_WIDTH_THRESHOLD,
    DEFAULT_MOUSE_SCROLL_LINES, DEFAULT_SCROLLBACK_LIMIT_BYTES,
};

pub const MAX_TOAST_DELAY_SECONDS: u64 = 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannelConfig {
    #[default]
    Stable,
    Preview,
}

impl UpdateChannelConfig {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Preview => "preview",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    pub channel: UpdateChannelConfig,
    pub version_check: bool,
    pub manifest_check: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            channel: default_update_channel(),
            version_check: true,
            manifest_check: true,
        }
    }
}

fn default_update_channel() -> UpdateChannelConfig {
    default_update_channel_for_build(cfg!(windows), crate::build_info::is_preview())
}

fn default_update_channel_for_build(is_windows: bool, is_preview: bool) -> UpdateChannelConfig {
    if is_windows && is_preview {
        UpdateChannelConfig::Preview
    } else {
        UpdateChannelConfig::Stable
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToastDelivery {
    #[default]
    Off,
    Herdr,
    Terminal,
    System,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "kebab-case")]
pub enum ToastHerdrPosition {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToastHerdrSize {
    /// Size the toast to its text content.
    #[default]
    Auto,
    /// At least 40% of the anchor area width, with inner padding.
    Medium,
    /// At least 60% of the anchor area width, with inner padding.
    Large,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ToastClipboardPosition {
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    #[default]
    BottomCenter,
    BottomRight,
    /// Centered in the pane the text was copied from; bottom-center when
    /// there is no such pane in view or it is too small (fork issue 129).
    Pane,
}

/// Where herdr's notes about a pane action (a refused or failed pane move,
/// clear scrollback, todo save) show (fork issue 129).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToastPaneFeedback {
    /// In the `[ui.toast.herdr]` position, like every other toast.
    #[default]
    Corner,
    /// Centered in the pane acted on; the corner when that pane is not in
    /// view or is too small.
    Pane,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentPanelSortConfig {
    #[default]
    #[serde(alias = "workspaces")]
    Spaces,
    Priority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LegacyAgentPanelScopeConfig {
    Current,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StatusIndicatorStyle {
    #[default]
    Dots,
    Symbols,
}

impl StatusIndicatorStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dots => "dots",
            Self::Symbols => "symbols",
        }
    }
}

/// Whether the working state icon in the sidebar's agent rows animates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StatusSpinnerConfig {
    /// Step a spinner on a slow shared tick while any agent is working. The
    /// tick is armed only while a working agent is on screen, so an idle
    /// session costs nothing.
    #[default]
    #[serde(alias = "agent")]
    On,
    /// Always draw the static working glyph.
    Off,
}

/// Bounds for `ui.status_spinner_ms`: fast enough to read as motion, slow
/// enough that the tick stays a rounding error next to PTY output.
pub const MIN_STATUS_SPINNER_MS: u64 = 50;
pub const MAX_STATUS_SPINNER_MS: u64 = 2000;
pub const DEFAULT_STATUS_SPINNER_MS: u64 = 200;

/// Bounds for `ui.display_panes_ms`: long enough to read the labels, short
/// enough that they never outstay the moment they answer.
pub const MIN_DISPLAY_PANES_MS: u64 = 500;
pub const MAX_DISPLAY_PANES_MS: u64 = 60_000;
pub const DEFAULT_DISPLAY_PANES_MS: u64 = 3000;

/// `ui.display_panes_ms` held to its bounds.
pub fn clamp_display_panes_ms(ms: u64) -> u64 {
    ms.clamp(MIN_DISPLAY_PANES_MS, MAX_DISPLAY_PANES_MS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceSortConfig {
    #[default]
    Manual,
    Priority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SidebarStyleConfig {
    /// Current layout: jump numbers lead the second row, bold headers.
    #[default]
    Default,
    /// Numbers right-aligned on the name row, thin uppercase headers,
    /// dimmed inactive meta lines.
    Editorial,
}

/// Optional color overrides for the sidebar state glyphs and state text.
/// Unset values fall back to the theme palette slots.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct StateColorsConfig {
    pub working: Option<String>,
    pub idle: Option<String>,
    pub done: Option<String>,
    pub blocked: Option<String>,
    pub unknown: Option<String>,
    /// Agents parked at their prompt behind work they launched. Unset follows
    /// `working`, which is the state they are still reported in.
    pub background: Option<String>,
}

/// Per-state override glyphs for the sidebar state icons. Unset values fall
/// back to the glyph the active `ui.status_indicators` style draws for that
/// state. A value must be exactly one terminal cell wide so the icon column
/// never shifts; anything else is reported as a diagnostic and ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct StateSymbolsConfig {
    pub working: Option<String>,
    pub idle: Option<String>,
    pub done: Option<String>,
    pub blocked: Option<String>,
    pub unknown: Option<String>,
    /// The two frames of the background-work pulse, in order.
    pub background: Option<String>,
    pub background_alt: Option<String>,
}

impl StateSymbolsConfig {
    fn entries(&self) -> [(&'static str, Option<&str>); 7] {
        [
            ("working", self.working.as_deref()),
            ("idle", self.idle.as_deref()),
            ("done", self.done.as_deref()),
            ("blocked", self.blocked.as_deref()),
            ("unknown", self.unknown.as_deref()),
            ("background", self.background.as_deref()),
            ("background_alt", self.background_alt.as_deref()),
        ]
    }

    /// The override when it is a usable single-cell glyph.
    pub fn valid(value: &Option<String>) -> Option<&str> {
        value.as_deref().filter(|glyph| is_single_cell(glyph))
    }

    pub fn diagnostics(&self) -> Vec<String> {
        self.entries()
            .into_iter()
            .filter_map(|(name, value)| {
                let value = value?;
                (!is_single_cell(value)).then(|| {
                    format!(
                        "ui.state_symbols.{name} = {value:?} must be exactly one terminal cell wide; ignoring"
                    )
                })
            })
            .collect()
    }
}

fn is_single_cell(glyph: &str) -> bool {
    unicode_width::UnicodeWidthStr::width(glyph) == 1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortMotionEasingConfig {
    /// Every bubble step is `sort_motion_step_ms` apart.
    #[default]
    Linear,
    /// Ease across a reshuffle: slow to break away, quickest mid-flight,
    /// slowing again as the list settles.
    Bubble,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortMotionConfig {
    /// Priority re-sorts settle for a delay, then rows bubble one position
    /// per step interval.
    #[default]
    Bubble,
    /// Priority re-sorts apply immediately (pre-motion behavior).
    Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum NotificationCenterPositionConfig {
    /// Anchored under the tab bar's right edge.
    #[default]
    TopRight,
    /// Anchored to the bottom-right of the frame.
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneBorderActiveStyleConfig {
    #[default]
    Light,
    Heavy,
    Double,
}

/// How the spaces list and the agent panel show what is scrolled out of view
/// (fork issue 159): summary rows at the edge, a lighter background on the
/// rows next to it, both, or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarOverflowConfig {
    #[default]
    Both,
    Rows,
    Fog,
    Off,
}

impl SidebarOverflowConfig {
    /// Summary rows at the edge of a list with rows hidden past it.
    pub fn edge_rows(self) -> bool {
        matches!(self, Self::Both | Self::Rows)
    }

    /// A lighter background on the rows next to a hidden edge.
    pub fn fog(self) -> bool {
        matches!(self, Self::Both | Self::Fog)
    }
}

/// How the fog next to a hidden sidebar edge shows (fork issue 166): a lighter
/// background, text faded toward the background, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarFogStyle {
    #[default]
    Lift,
    Dim,
    Both,
}

impl SidebarFogStyle {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "lift" => Some(Self::Lift),
            "dim" => Some(Self::Dim),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    /// The background of the fogged rows is lifted.
    pub fn lifts(self) -> bool {
        matches!(self, Self::Lift | Self::Both)
    }

    /// The text of the fogged rows is faded toward the background.
    pub fn dims(self) -> bool {
        matches!(self, Self::Dim | Self::Both)
    }
}

/// Highlight pattern for the active space/agent in the sidebar. Accepts a
/// string mode or, for backward compatibility, a bool (true = both).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarActiveBorderConfig {
    #[default]
    Off,
    Above,
    Below,
    Both,
    Left,
    Right,
}

impl SidebarActiveBorderConfig {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Above => "above",
            Self::Below => "below",
            Self::Both => "both",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

impl Serialize for SidebarActiveBorderConfig {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SidebarActiveBorderConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ModeVisitor;

        impl serde::de::Visitor<'_> for ModeVisitor {
            type Value = SidebarActiveBorderConfig;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(
                    "a bool or one of \"off\", \"above\", \"below\", \"both\", \"left\", \"right\"",
                )
            }

            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(if value {
                    SidebarActiveBorderConfig::Both
                } else {
                    SidebarActiveBorderConfig::Off
                })
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                match value {
                    "off" => Ok(SidebarActiveBorderConfig::Off),
                    "above" => Ok(SidebarActiveBorderConfig::Above),
                    "below" => Ok(SidebarActiveBorderConfig::Below),
                    "both" => Ok(SidebarActiveBorderConfig::Both),
                    "left" => Ok(SidebarActiveBorderConfig::Left),
                    "right" => Ok(SidebarActiveBorderConfig::Right),
                    _ => Err(E::unknown_variant(
                        value,
                        &["off", "above", "below", "both", "left", "right"],
                    )),
                }
            }
        }

        deserializer.deserialize_any(ModeVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HostCursorModeConfig {
    #[default]
    Auto,
    Native,
    Drawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarCollapsedModeConfig {
    #[default]
    Compact,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RightClickPassthroughModifierConfig(Option<KeyModifiers>);

impl RightClickPassthroughModifierConfig {
    pub fn modifiers(self) -> Option<KeyModifiers> {
        self.0
    }
}

impl<'de> Deserialize<'de> for RightClickPassthroughModifierConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        parse_right_click_passthrough_modifier(&value)
            .map(Self)
            .ok_or_else(|| {
                de::Error::custom(
                    "right_click_passthrough_modifier must be empty, off, none, disabled, ctrl/control, alt/option, cmd/command/super, meta, hyper, or a + separated combination without shift",
                )
            })
    }
}

fn parse_right_click_passthrough_modifier(value: &str) -> Option<Option<KeyModifiers>> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("off")
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("disabled")
    {
        return Some(None);
    }

    let mut modifiers = KeyModifiers::empty();
    for token in trimmed.split('+') {
        let token = token.trim().to_ascii_lowercase();
        let modifier = match token.as_str() {
            "ctrl" | "control" => KeyModifiers::CONTROL,
            "alt" | "option" => KeyModifiers::ALT,
            "cmd" | "command" | "super" => KeyModifiers::SUPER,
            "meta" => KeyModifiers::META,
            "hyper" => KeyModifiers::HYPER,
            "shift" => return None,
            _ => return None,
        };
        modifiers |= modifier;
    }

    (!modifiers.is_empty()).then_some(Some(modifiers))
}

#[derive(Debug, Clone)]
pub struct ToastConfig {
    pub delivery: ToastDelivery,
    pub delay_seconds: u64,
    pub herdr: HerdrToastConfig,
    pub clipboard: ClipboardToastConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct HerdrToastConfig {
    pub position: ToastHerdrPosition,
    /// Toast box size preset: "auto" hugs the text, "medium" and "large"
    /// widen the box relative to the anchor area. Default: auto.
    pub size: ToastHerdrSize,
    /// How long a needs-attention toast stays visible, in seconds. 0 keeps it
    /// visible until clicked or replaced. Default: 8.
    pub needs_attention_seconds: u64,
    /// How long a finished toast stays visible, in seconds. 0 keeps it
    /// visible until clicked or replaced. Default: 5.
    pub finished_seconds: u64,
    /// How long an update-installed toast stays visible, in seconds. 0 keeps
    /// it visible until clicked or replaced. Default: 3.
    pub update_seconds: u64,
    /// Where notes about a pane action show: "corner" (the position above)
    /// or "pane" (centered in the pane acted on). Default: corner.
    pub pane_feedback: ToastPaneFeedback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ClipboardToastConfig {
    pub enabled: bool,
    pub position: ToastClipboardPosition,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NewTerminalCwdConfig {
    #[default]
    Follow,
    Home,
    Current,
    Path(String),
}

impl<'de> Deserialize<'de> for NewTerminalCwdConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.trim() {
            "" | "follow" => Ok(Self::Follow),
            "home" => Ok(Self::Home),
            "current" => Ok(Self::Current),
            _ => Ok(Self::Path(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellModeConfig {
    #[default]
    Auto,
    Login,
    NonLogin,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// Executable used for new interactive panes. Empty means SHELL, then /bin/sh.
    pub default_shell: String,
    /// Startup mode for new interactive pane shells.
    pub shell_mode: ShellModeConfig,
    /// CWD policy for new interactive panes, tabs, and workspaces.
    pub new_cwd: NewTerminalCwdConfig,
    /// Render Kitty graphics in compatible outer terminals. Default: true.
    pub kitty_graphics: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    /// Resume supported AI-agent panes into their native conversation sessions
    /// when restoring a Herdr session. Default: true.
    pub resume_agents_on_restore: bool,
    /// Milliseconds between automatic agent restores. Zero disables spacing.
    pub startup_per_agent_delay_ms: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            resume_agents_on_restore: true,
            startup_per_agent_delay_ms: 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigReloadStatus {
    Applied,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConfigReloadReport {
    pub status: ConfigReloadStatus,
    pub diagnostics: Vec<String>,
}

/// Validate `[ui]` sidebar bound configuration.
///
/// Returns `Some((min, max))` when `min <= max`, `None` otherwise. The two
/// values are funneled through this helper before they reach any
/// `u16::clamp(min, max)` call site (`u16::clamp` panics when `min > max`).
pub fn validated_sidebar_bounds(min: u16, max: u16) -> Option<(u16, u16)> {
    if min <= max {
        Some((min, max))
    } else {
        None
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub onboarding: Option<bool>,
    pub theme: ThemeConfig,
    pub terminal: TerminalConfig,
    pub session: SessionConfig,
    pub server: ServerConfig,
    pub update: UpdateConfig,
    pub keys: KeysConfig,
    pub ui: UiConfig,
    pub worktrees: WorktreesConfig,
    pub advanced: AdvancedConfig,
    pub experimental: ExperimentalConfig,
    pub remote: RemoteConfig,
    pub agents: AgentsConfig,
}

#[derive(Debug)]
pub struct LoadedConfig {
    pub config: Config,
    pub diagnostics: Vec<String>,
    pub invalid_sections: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeysConfig {
    /// Prefix key(s) to enter prefix mode (e.g. "ctrl+b", "f12", "esc", or an
    /// array to accept several).
    pub prefix: BindingConfig,
    /// Open keybinding help. Default: "prefix+?"
    pub help: BindingConfig,
    /// Open settings. Default: "prefix+s"
    pub settings: BindingConfig,
    /// Create a new workspace. Default: "prefix+shift+n"
    pub new_workspace: BindingConfig,
    /// Create a Git worktree from the selected workspace. Default: "prefix+shift+g"
    pub new_worktree: BindingConfig,
    /// Open an existing Git worktree from the selected workspace. Unset by default.
    pub open_worktree: BindingConfig,
    /// Delete the selected managed worktree checkout after confirmation. Unset by default.
    pub remove_worktree: BindingConfig,
    /// Rename the selected workspace. Default: "prefix+shift+w"
    pub rename_workspace: BindingConfig,
    /// Pin or unpin the selected workspace to the top of the list. Default: unset
    pub toggle_pin_workspace: BindingConfig,
    /// Type into every pane of the current tab at once. Default: "prefix+shift+s"
    pub toggle_sync_panes: BindingConfig,
    /// Pin or unpin the focused pane's agent to the top of the agent panel. Default: unset
    pub toggle_pin_agent: BindingConfig,
    /// Close the selected workspace. Default: "prefix+shift+d"
    pub close_workspace: BindingConfig,
    /// Open the workspace navigation surface. Default: "prefix+w"
    pub workspace_picker: BindingConfig,
    /// Open the session navigator. Default: "prefix+g"
    pub goto: BindingConfig,
    /// Move workspace selection up in navigate mode. Default: "up".
    pub navigate_workspace_up: BindingConfig,
    /// Move workspace selection down in navigate mode. Default: "down".
    pub navigate_workspace_down: BindingConfig,
    /// Focus the pane to the left in navigate mode. Default: "h". Left arrow is always an alias.
    pub navigate_pane_left: BindingConfig,
    /// Focus the pane below in navigate mode. Default: "j".
    pub navigate_pane_down: BindingConfig,
    /// Focus the pane above in navigate mode. Default: "k".
    pub navigate_pane_up: BindingConfig,
    /// Focus the pane to the right in navigate mode. Default: "l". Right arrow is always an alias.
    pub navigate_pane_right: BindingConfig,
    /// Detach the current client from its Herdr server. Default: "prefix+q".
    pub detach: BindingConfig,
    /// Reload config.toml in the running app/server. Default: "prefix+shift+r".
    pub reload_config: BindingConfig,
    /// Focus the currently visible notification target. Default: "prefix+o".
    pub open_notification_target: BindingConfig,
    /// Open the notification center panel. Default: "prefix+ctrl+n".
    pub open_notification_center: BindingConfig,
    /// Open the focused pane's todo panel. Default: "prefix+ctrl+t".
    pub open_pane_todos: BindingConfig,
    /// Open the todo editor on a new todo for the focused pane. Unbound by default.
    pub add_pane_todo: BindingConfig,
    /// Open the session-wide todo board. Unbound by default.
    pub open_todo_board: BindingConfig,
    /// Show every pane's number, address, name and size in characters, and the
    /// window size, until the next key or for 3 seconds. Default: "prefix+i".
    pub display_panes: BindingConfig,
    /// Select the previous workspace. Unset by default.
    pub previous_workspace: BindingConfig,
    /// Select the next workspace. Unset by default.
    pub next_workspace: BindingConfig,
    /// Focus the previous agent shown in the agent panel. Unset by default.
    pub previous_agent: BindingConfig,
    /// Focus the next agent shown in the agent panel. Unset by default.
    pub next_agent: BindingConfig,
    /// Focus an agent by index 1-9. Unset by default.
    pub focus_agent: BindingConfig,
    /// Local-client shortcut that sends a clipboard image to a remote Herdr session. Default: "ctrl+v".
    pub remote_image_paste: String,
    /// Create a new tab in the active workspace. Default: "prefix+c"
    pub new_tab: BindingConfig,
    /// Rename the active tab. Default: "prefix+shift+t".
    pub rename_tab: BindingConfig,
    /// Select the previous tab. Default: "prefix+p".
    pub previous_tab: BindingConfig,
    /// Select the next tab. Default: "prefix+n".
    pub next_tab: BindingConfig,
    /// Move the active tab one position toward the front. Unset by default.
    pub move_tab_previous: BindingConfig,
    /// Move the active tab one position toward the back. Unset by default.
    pub move_tab_next: BindingConfig,
    /// Switch to tab 1-9. Default: "prefix+1..9".
    pub switch_tab: BindingConfig,
    /// Switch to workspace 1-9 from prefix mode. Unset by default.
    pub switch_workspace: BindingConfig,
    /// Close the active tab. Default: "prefix+shift+x".
    pub close_tab: BindingConfig,
    /// Rename the focused pane. Default: "prefix+shift+p".
    pub rename_pane: BindingConfig,
    /// Break the focused pane into a new tab. Default: "prefix+!".
    pub break_pane: BindingConfig,
    /// Move the focused pane to another tab or space via a picker. Default: "prefix+m".
    pub move_pane_to_tab: BindingConfig,
    /// Move the focused pane to the next tab without wrapping. Default: "prefix+>".
    pub move_pane_next_tab: BindingConfig,
    /// Move the focused pane to the previous tab without wrapping. Default: "prefix+<".
    pub move_pane_prev_tab: BindingConfig,
    /// Open the focused pane scrollback in $EDITOR. Default: "prefix+e".
    pub edit_scrollback: BindingConfig,
    pub clear_pane: BindingConfig,
    /// Purge the focused pane's saved scrollback (tmux `clear-history`).
    /// Unbound by default.
    pub clear_scrollback: BindingConfig,
    /// Enter keyboard copy mode for the focused pane. Default: "prefix+[".
    pub copy_mode: BindingConfig,
    /// Enter copy mode and scroll up one page in the same gesture
    /// (tmux `copy-mode -u`). Default: "prefix+pageup".
    pub copy_mode_page_up: BindingConfig,
    /// Enter copy mode and scroll up half a page. Default: "prefix+ctrl+u".
    pub copy_mode_half_page_up: BindingConfig,
    /// Enter copy mode and scroll the viewport up one line. Default: "prefix+ctrl+k".
    pub copy_mode_line_up: BindingConfig,
    /// Scroll the focused pane down a page. Default: "prefix+pagedown"
    pub copy_mode_page_down: BindingConfig,
    /// Scroll the focused pane down half a page. Default: "prefix+ctrl+d"
    pub copy_mode_half_page_down: BindingConfig,
    /// Scroll the focused pane down one line. Default: "prefix+ctrl+j"
    pub copy_mode_line_down: BindingConfig,
    /// Focus the pane to the left. Default: "prefix+h".
    pub focus_pane_left: BindingConfig,
    /// Focus the pane below. Default: "prefix+j".
    pub focus_pane_down: BindingConfig,
    /// Focus the pane above. Default: "prefix+k".
    pub focus_pane_up: BindingConfig,
    /// Focus the pane to the right. Default: "prefix+l".
    pub focus_pane_right: BindingConfig,
    /// Swap the focused pane with the pane to the left. Default: "prefix+shift+h".
    pub swap_pane_left: BindingConfig,
    /// Swap the focused pane with the pane below. Default: "prefix+shift+j".
    pub swap_pane_down: BindingConfig,
    /// Swap the focused pane with the pane above. Default: "prefix+shift+k".
    pub swap_pane_up: BindingConfig,
    /// Swap the focused pane with the pane to the right. Default: "prefix+shift+l".
    pub swap_pane_right: BindingConfig,
    /// Cycle to the next pane. Default: "prefix+tab".
    pub cycle_pane_next: BindingConfig,
    /// Cycle to the previous pane. Default: "prefix+shift+tab".
    pub cycle_pane_previous: BindingConfig,
    /// Focus the last focused pane across workspaces and tabs. Unset by default.
    pub last_pane: BindingConfig,
    /// Split pane vertically (side by side). Default: "prefix+v"
    pub split_vertical: BindingConfig,
    /// Split pane horizontally (stacked). Default: "prefix+minus"
    pub split_horizontal: BindingConfig,
    /// Close the focused pane. Default: "prefix+x"
    pub close_pane: BindingConfig,
    /// Restart the focused pane's process in place. Default: "prefix+ctrl+x"
    pub respawn_pane: BindingConfig,
    /// Toggle zoom for the focused pane. Default: "prefix+z"
    #[serde(alias = "fullscreen")]
    pub zoom: BindingConfig,
    /// Enter resize mode. Default: "prefix+r"
    pub resize_mode: BindingConfig,
    /// Resize the focused pane toward the left. Unset by default.
    pub resize_pane_left: BindingConfig,
    /// Resize the focused pane downward. Unset by default.
    pub resize_pane_down: BindingConfig,
    /// Resize the focused pane upward. Unset by default.
    pub resize_pane_up: BindingConfig,
    /// Resize the focused pane toward the right. Unset by default.
    pub resize_pane_right: BindingConfig,
    /// Balance all panes in the current tab to equal sizes. Default: "prefix+="
    pub balance_panes: BindingConfig,
    /// Cycle the tab through layout presets (even-h -> even-v -> tiled). Default: "prefix+space"
    pub next_layout: BindingConfig,
    /// Toggle sidebar collapse. Default: "prefix+b"
    pub toggle_sidebar: BindingConfig,
    /// Optional indexed shortcuts expanded over number keys 1-9.
    pub indexed: IndexedKeysConfig,
    /// Prefix-mode custom command bindings.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<CommandKeybindConfig>,
    #[serde(skip_serializing)]
    pub(crate) user_fields: BTreeSet<&'static str>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct KeysConfigOverlay {
    #[serde(skip_serializing_if = "Option::is_none")]
    prefix: Option<BindingConfig>,
    /// Additional prefix keys published for cross-version compatibility.
    /// Older clients parse `prefix` as a single string and ignore this field;
    /// new clients merge it into the effective prefix list.
    #[serde(skip_serializing_if = "Option::is_none")]
    extra_prefixes: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    help: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_workspace: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_worktree: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_worktree: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remove_worktree: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rename_workspace: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    toggle_pin_workspace: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    toggle_sync_panes: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    toggle_pin_agent: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    close_workspace: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_picker: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    goto: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    navigate_workspace_up: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    navigate_workspace_down: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    navigate_pane_left: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    navigate_pane_down: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    navigate_pane_up: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    navigate_pane_right: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detach: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reload_config: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_notification_target: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_notification_center: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_pane_todos: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    add_pane_todo: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_todo_board: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_panes: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_workspace: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_workspace: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_agent: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_agent: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    focus_agent: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_image_paste: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_tab: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rename_tab: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_tab: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_tab: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    move_tab_previous: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    move_tab_next: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    switch_tab: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    switch_workspace: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    close_tab: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rename_pane: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    break_pane: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    move_pane_to_tab: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    move_pane_next_tab: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    move_pane_prev_tab: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    edit_scrollback: Option<BindingConfig>,
    clear_pane: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clear_scrollback: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    copy_mode: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    copy_mode_page_up: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    copy_mode_half_page_up: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    copy_mode_line_up: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    copy_mode_page_down: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    copy_mode_half_page_down: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    copy_mode_line_down: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    focus_pane_left: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    focus_pane_down: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    focus_pane_up: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    focus_pane_right: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    swap_pane_left: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    swap_pane_down: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    swap_pane_up: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    swap_pane_right: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cycle_pane_next: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cycle_pane_previous: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_pane: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    split_vertical: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    split_horizontal: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    close_pane: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    respawn_pane: Option<BindingConfig>,
    #[serde(alias = "fullscreen", skip_serializing_if = "Option::is_none")]
    zoom: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resize_mode: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resize_pane_left: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resize_pane_down: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resize_pane_up: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resize_pane_right: Option<BindingConfig>,
    balance_panes: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_layout: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    toggle_sidebar: Option<BindingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    indexed: Option<IndexedKeysConfig>,
    #[serde(skip_serializing)]
    command: Option<Vec<CommandKeybindConfig>>,
}

impl KeysConfigOverlay {
    pub(crate) fn set_prefixes(&mut self, prefixes: &[super::keybinds::KeyCombo]) {
        let mut labels = prefixes
            .iter()
            .map(|combo| super::keybinds::format_key_combo(*combo));
        self.prefix = Some(BindingConfig::One(labels.next().unwrap_or_default()));
        let extra: Vec<String> = labels.collect();
        self.extra_prefixes = (!extra.is_empty()).then_some(BindingConfig::Many(extra));
    }
}

impl<'de> Deserialize<'de> for KeysConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = KeysConfigOverlay::deserialize(deserializer)?;
        let mut keys = KeysConfig::default();

        let prefix_was_supplied = input.prefix.is_some() || input.extra_prefixes.is_some();
        let mut prefix_values = Vec::new();
        if let Some(prefix) = input.prefix {
            prefix_values.extend(prefix.into_values());
        }
        if let Some(extra) = input.extra_prefixes {
            prefix_values.extend(extra.into_values());
        }
        if prefix_was_supplied {
            // An explicitly empty list stays empty so prefix validation rejects
            // it and a reload keeps the current keybindings.
            keys.prefix = match prefix_values.len() {
                0 => BindingConfig::Many(Vec::new()),
                1 => BindingConfig::One(prefix_values.remove(0)),
                _ => BindingConfig::Many(prefix_values),
            };
            keys.user_fields.insert("prefix");
        }

        macro_rules! apply_field {
            ($field:ident) => {
                if let Some(value) = input.$field {
                    keys.$field = value;
                    keys.user_fields.insert(stringify!($field));
                }
            };
        }

        apply_field!(help);
        apply_field!(settings);
        apply_field!(new_workspace);
        apply_field!(new_worktree);
        apply_field!(open_worktree);
        apply_field!(remove_worktree);
        apply_field!(rename_workspace);
        apply_field!(toggle_pin_workspace);
        apply_field!(toggle_sync_panes);
        apply_field!(toggle_pin_agent);
        apply_field!(close_workspace);
        apply_field!(workspace_picker);
        apply_field!(goto);
        apply_field!(navigate_workspace_up);
        apply_field!(navigate_workspace_down);
        apply_field!(navigate_pane_left);
        apply_field!(navigate_pane_down);
        apply_field!(navigate_pane_up);
        apply_field!(navigate_pane_right);
        apply_field!(detach);
        apply_field!(reload_config);
        apply_field!(open_notification_target);
        apply_field!(open_notification_center);
        apply_field!(open_pane_todos);
        apply_field!(add_pane_todo);
        apply_field!(open_todo_board);
        apply_field!(display_panes);
        apply_field!(previous_workspace);
        apply_field!(next_workspace);
        apply_field!(previous_agent);
        apply_field!(next_agent);
        apply_field!(focus_agent);
        apply_field!(remote_image_paste);
        apply_field!(new_tab);
        apply_field!(rename_tab);
        apply_field!(previous_tab);
        apply_field!(next_tab);
        apply_field!(move_tab_previous);
        apply_field!(move_tab_next);
        apply_field!(switch_tab);
        apply_field!(switch_workspace);
        apply_field!(close_tab);
        apply_field!(rename_pane);
        apply_field!(break_pane);
        apply_field!(move_pane_to_tab);
        apply_field!(move_pane_next_tab);
        apply_field!(move_pane_prev_tab);
        apply_field!(edit_scrollback);
        apply_field!(clear_pane);
        apply_field!(clear_scrollback);
        apply_field!(copy_mode);
        apply_field!(copy_mode_page_up);
        apply_field!(copy_mode_half_page_up);
        apply_field!(copy_mode_line_up);
        apply_field!(copy_mode_page_down);
        apply_field!(copy_mode_half_page_down);
        apply_field!(copy_mode_line_down);
        apply_field!(focus_pane_left);
        apply_field!(focus_pane_down);
        apply_field!(focus_pane_up);
        apply_field!(focus_pane_right);
        apply_field!(swap_pane_left);
        apply_field!(swap_pane_down);
        apply_field!(swap_pane_up);
        apply_field!(swap_pane_right);
        apply_field!(cycle_pane_next);
        apply_field!(cycle_pane_previous);
        apply_field!(last_pane);
        apply_field!(split_vertical);
        apply_field!(split_horizontal);
        apply_field!(close_pane);
        apply_field!(respawn_pane);
        apply_field!(zoom);
        apply_field!(resize_mode);
        apply_field!(resize_pane_left);
        apply_field!(resize_pane_down);
        apply_field!(resize_pane_up);
        apply_field!(resize_pane_right);
        apply_field!(balance_panes);
        apply_field!(next_layout);
        apply_field!(toggle_sidebar);
        apply_field!(indexed);
        apply_field!(command);

        Ok(keys)
    }
}

impl KeysConfig {
    pub(crate) fn key_field_is_user_configured(&self, field: &str) -> bool {
        self.user_fields.contains(field)
    }

    pub(crate) fn local_profile(&self, keybinds: &Keybinds) -> KeysConfigOverlay {
        let mut profile = KeysConfigOverlay::default();

        macro_rules! copy_user_field {
            ($field:ident) => {
                if self.user_fields.contains(stringify!($field)) {
                    profile.$field = Some(self.$field.clone());
                }
            };
        }
        macro_rules! copy_effective_action_field {
            ($field:ident, $target:expr) => {
                if self.user_fields.contains(stringify!($field)) {
                    profile.$field = Some(self.$field.clone());
                } else if binding_config_is_effective(&self.$field, &$target) {
                    profile.$field = Some(self.$field.clone());
                } else if binding_config_has_values(&self.$field) {
                    profile.$field = Some(BindingConfig::empty());
                }
            };
        }
        macro_rules! copy_effective_indexed_field {
            ($field:ident, $target:expr) => {
                if self.user_fields.contains(stringify!($field)) {
                    profile.$field = Some(self.$field.clone());
                } else if let Some(effective) = effective_indexed_config(&self.$field, &$target) {
                    profile.$field = Some(effective);
                } else if binding_config_has_values(&self.$field) {
                    profile.$field = Some(BindingConfig::empty());
                }
            };
        }

        profile.prefix = Some(self.prefix.clone());
        copy_effective_action_field!(help, keybinds.help);
        copy_effective_action_field!(settings, keybinds.settings);
        copy_effective_action_field!(new_workspace, keybinds.new_workspace);
        copy_effective_action_field!(new_worktree, keybinds.new_worktree);
        copy_effective_action_field!(open_worktree, keybinds.open_worktree);
        copy_effective_action_field!(remove_worktree, keybinds.remove_worktree);
        copy_effective_action_field!(rename_workspace, keybinds.rename_workspace);
        copy_effective_action_field!(toggle_pin_workspace, keybinds.toggle_pin_workspace);
        copy_effective_action_field!(toggle_sync_panes, keybinds.toggle_sync_panes);
        copy_effective_action_field!(toggle_pin_agent, keybinds.toggle_pin_agent);
        copy_effective_action_field!(close_workspace, keybinds.close_workspace);
        copy_effective_action_field!(workspace_picker, keybinds.workspace_picker);
        copy_effective_action_field!(goto, keybinds.goto);
        copy_effective_action_field!(navigate_workspace_up, keybinds.navigate.workspace_up);
        copy_effective_action_field!(navigate_workspace_down, keybinds.navigate.workspace_down);
        copy_effective_action_field!(navigate_pane_left, keybinds.navigate.pane_left);
        copy_effective_action_field!(navigate_pane_down, keybinds.navigate.pane_down);
        copy_effective_action_field!(navigate_pane_up, keybinds.navigate.pane_up);
        copy_effective_action_field!(navigate_pane_right, keybinds.navigate.pane_right);
        copy_effective_action_field!(detach, keybinds.detach);
        copy_effective_action_field!(reload_config, keybinds.reload_config);
        copy_effective_action_field!(open_notification_target, keybinds.open_notification_target);
        copy_effective_action_field!(open_notification_center, keybinds.open_notification_center);
        copy_effective_action_field!(open_pane_todos, keybinds.open_pane_todos);
        copy_effective_action_field!(add_pane_todo, keybinds.add_pane_todo);
        copy_effective_action_field!(open_todo_board, keybinds.open_todo_board);
        copy_effective_action_field!(display_panes, keybinds.display_panes);
        copy_effective_action_field!(previous_workspace, keybinds.previous_workspace);
        copy_effective_action_field!(next_workspace, keybinds.next_workspace);
        copy_effective_action_field!(previous_agent, keybinds.previous_agent);
        copy_effective_action_field!(next_agent, keybinds.next_agent);
        copy_effective_indexed_field!(focus_agent, keybinds.focus_agent);
        copy_user_field!(remote_image_paste);
        copy_effective_action_field!(new_tab, keybinds.new_tab);
        copy_effective_action_field!(rename_tab, keybinds.rename_tab);
        copy_effective_action_field!(previous_tab, keybinds.previous_tab);
        copy_effective_action_field!(next_tab, keybinds.next_tab);
        copy_effective_action_field!(move_tab_previous, keybinds.move_tab_previous);
        copy_effective_action_field!(move_tab_next, keybinds.move_tab_next);
        copy_effective_indexed_field!(switch_tab, keybinds.switch_tab);
        copy_effective_indexed_field!(switch_workspace, keybinds.switch_workspace);
        copy_effective_action_field!(close_tab, keybinds.close_tab);
        copy_effective_action_field!(rename_pane, keybinds.rename_pane);
        copy_effective_action_field!(break_pane, keybinds.break_pane);
        copy_effective_action_field!(move_pane_to_tab, keybinds.move_pane_to_tab);
        copy_effective_action_field!(move_pane_next_tab, keybinds.move_pane_next_tab);
        copy_effective_action_field!(move_pane_prev_tab, keybinds.move_pane_prev_tab);
        copy_effective_action_field!(edit_scrollback, keybinds.edit_scrollback);
        copy_effective_action_field!(clear_pane, keybinds.clear_pane);
        copy_effective_action_field!(clear_scrollback, keybinds.clear_scrollback);
        copy_effective_action_field!(copy_mode, keybinds.copy_mode);
        copy_effective_action_field!(copy_mode_page_up, keybinds.copy_mode_page_up);
        copy_effective_action_field!(copy_mode_half_page_up, keybinds.copy_mode_half_page_up);
        copy_effective_action_field!(copy_mode_line_up, keybinds.copy_mode_line_up);
        copy_effective_action_field!(copy_mode_page_down, keybinds.copy_mode_page_down);
        copy_effective_action_field!(copy_mode_half_page_down, keybinds.copy_mode_half_page_down);
        copy_effective_action_field!(copy_mode_line_down, keybinds.copy_mode_line_down);
        copy_effective_action_field!(focus_pane_left, keybinds.focus_pane_left);
        copy_effective_action_field!(focus_pane_down, keybinds.focus_pane_down);
        copy_effective_action_field!(focus_pane_up, keybinds.focus_pane_up);
        copy_effective_action_field!(focus_pane_right, keybinds.focus_pane_right);
        copy_effective_action_field!(swap_pane_left, keybinds.swap_pane_left);
        copy_effective_action_field!(swap_pane_down, keybinds.swap_pane_down);
        copy_effective_action_field!(swap_pane_up, keybinds.swap_pane_up);
        copy_effective_action_field!(swap_pane_right, keybinds.swap_pane_right);
        copy_effective_action_field!(cycle_pane_next, keybinds.cycle_pane_next);
        copy_effective_action_field!(cycle_pane_previous, keybinds.cycle_pane_previous);
        copy_effective_action_field!(last_pane, keybinds.last_pane);
        copy_effective_action_field!(split_vertical, keybinds.split_vertical);
        copy_effective_action_field!(split_horizontal, keybinds.split_horizontal);
        copy_effective_action_field!(close_pane, keybinds.close_pane);
        copy_effective_action_field!(respawn_pane, keybinds.respawn_pane);
        copy_effective_action_field!(zoom, keybinds.zoom);
        copy_effective_action_field!(resize_mode, keybinds.resize_mode);
        copy_effective_action_field!(resize_pane_left, keybinds.resize_pane_left);
        copy_effective_action_field!(resize_pane_down, keybinds.resize_pane_down);
        copy_effective_action_field!(resize_pane_up, keybinds.resize_pane_up);
        copy_effective_action_field!(resize_pane_right, keybinds.resize_pane_right);
        copy_effective_action_field!(balance_panes, keybinds.balance_panes);
        copy_effective_action_field!(next_layout, keybinds.next_layout);
        copy_effective_action_field!(toggle_sidebar, keybinds.toggle_sidebar);
        copy_user_field!(indexed);

        profile
    }
}

fn binding_config_has_values(config: &BindingConfig) -> bool {
    config.has_values()
}

fn binding_config_is_effective(config: &BindingConfig, keybinds: &ActionKeybinds) -> bool {
    !binding_config_has_values(config) || !keybinds.bindings.is_empty()
}

fn effective_indexed_config(
    config: &BindingConfig,
    keybinds: &[IndexedKeybind],
) -> Option<BindingConfig> {
    if !binding_config_has_values(config) {
        return Some(config.clone());
    }

    let expected_labels = config.indexed_labels();
    if expected_labels.is_empty() {
        return None;
    }

    let effective_labels: Vec<String> = expected_labels
        .iter()
        .filter(|expected| {
            keybinds
                .iter()
                .any(|binding| binding.label.as_str() == expected.as_str())
        })
        .cloned()
        .collect();

    if effective_labels.is_empty() {
        None
    } else if effective_labels.len() == expected_labels.len() {
        Some(config.clone())
    } else {
        Some(BindingConfig::Many(effective_labels))
    }
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct IndexedKeysConfig {
    /// Modifier combo for tab shortcuts 1-9. Unset by default.
    pub tabs: String,
    /// Modifier combo for workspace shortcuts 1-9. Unset by default.
    pub workspaces: String,
    /// Modifier combo for agent shortcuts 1-9. Unset by default.
    pub agents: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct WorktreesConfig {
    /// Root directory under which Herdr creates <repo>/<branch-slug> checkouts.
    pub directory: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TabBarPositionConfig {
    #[default]
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneBordersConfig {
    #[default]
    Auto,
    Always,
    Off,
}

impl PaneBordersConfig {
    pub fn draws_borders(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn shows_borders(self, multi_pane: bool) -> bool {
        self.draws_borders() && (multi_pane || matches!(self, Self::Always))
    }
}

impl<'de> Deserialize<'de> for PaneBordersConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PaneBordersVisitor;

        impl<'de> de::Visitor<'de> for PaneBordersVisitor {
            type Value = PaneBordersConfig;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("\"auto\", \"always\", \"off\", or a legacy boolean")
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(if value {
                    PaneBordersConfig::Auto
                } else {
                    PaneBordersConfig::Off
                })
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                match value {
                    "auto" => Ok(PaneBordersConfig::Auto),
                    "always" => Ok(PaneBordersConfig::Always),
                    "off" => Ok(PaneBordersConfig::Off),
                    other => Err(E::invalid_value(de::Unexpected::Str(other), &self)),
                }
            }
        }

        deserializer.deserialize_any(PaneBordersVisitor)
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub sidebar_width: u16,
    /// Minimum sidebar width (columns) when expanded. Default: 18.
    pub sidebar_min_width: u16,
    /// Maximum sidebar width (columns) when expanded. Default: 36.
    pub sidebar_max_width: u16,
    /// Start with the sidebar collapsed. Default: false.
    pub sidebar_start_collapsed: bool,
    /// Collapsed sidebar presentation. Default: compact.
    pub sidebar_collapsed_mode: SidebarCollapsedModeConfig,
    /// Terminal width at or below which Herdr uses the mobile single-column layout. Default: 64.
    pub mobile_width_threshold: u16,
    /// Capture mouse input for Herdr's mouse UI. Default: true.
    pub mouse_capture: bool,
    /// Copy text selected with the mouse. Default: true.
    pub copy_on_select: bool,
    /// Host cursor policy. Default: auto.
    pub host_cursor: HostCursorModeConfig,
    /// Modifier that lets right-click gestures pass through to pane apps. Empty disables it.
    pub right_click_passthrough_modifier: RightClickPassthroughModifierConfig,
    /// Force a full host-terminal redraw when the outer terminal regains focus. Default: true.
    pub redraw_on_focus_gained: bool,
    /// Lines to scroll per mouse wheel notch. Default: 3.
    pub mouse_scroll_lines: Option<NonZeroUsize>,
    /// Ask for confirmation before closing a workspace. Default: true.
    pub confirm_close: bool,
    /// Ask for a tab name before creating a new tab. Default: true.
    pub prompt_new_tab_name: bool,
    /// Ask for a workspace name before interactive creation. Default: false.
    pub prompt_new_workspace_name: bool,
    /// Draw borders around split panes. auto draws them only for split panes,
    /// always also frames a lone pane (only while pane_outer_borders is
    /// enabled, since every edge of a lone pane is an outer edge), off
    /// disables them. Legacy booleans map true to auto and false to off.
    /// Default: auto.
    pub pane_borders: PaneBordersConfig,
    /// Draw borders along the outside edge of the pane area. Default: true.
    pub pane_outer_borders: bool,
    /// Draw interactive scrollbars beside terminal panes. Default: true.
    pub pane_scrollbars: bool,
    /// Keep split panes visually separated instead of sharing divider borders. Default: true.
    pub pane_gaps: bool,
    /// Show agent labels in split pane borders when no manual pane label is set. Default: false.
    pub show_agent_labels_on_pane_borders: bool,
    /// Show a todo indicator at the far right of a split pane's top border,
    /// carrying the pane's outstanding todo count. Default: true.
    pub show_pane_todo_indicator: bool,
    /// Hide the tab row when the workspace has one tab. Default: false.
    pub hide_tab_bar_when_single_tab: bool,
    /// Desktop tab row placement. Default: top.
    pub tab_bar_position: TabBarPositionConfig,
    /// Ordered entries shown at the right edge of the desktop tab row. Empty by default.
    pub tab_bar_right: Vec<TabBarRightEntryConfig>,
    /// Text inserted between visible right-side tab bar entries. Default: one space.
    pub tab_bar_right_separator: String,
    /// Format for the outer terminal window title. Empty leaves the title alone.
    /// Default: "{hostname}: {workspace}".
    pub window_title: String,
    /// Show the workspace jump number (1-9, the `switch_workspace` target) on the
    /// sidebar branch line. Default: false.
    /// Show the workspace jump symbol (1-9, then a-z; the `switch_workspace`
    /// target) on the sidebar branch line. Default: false.
    pub show_workspace_numbers: bool,
    /// Show each agent's jump symbol (1-9, then a-z; the `focus_agent`
    /// target) on the agent panel status line. Default: false.
    pub show_agent_numbers: bool,
    /// Show the Herdr server's short host name, right-aligned on the sidebar
    /// "SPACES" header row. Default: true.
    pub show_host: bool,
    /// Agent sidebar ordering. Saved values are "spaces" or "priority". Default: "spaces".
    pub agent_panel_sort: AgentPanelSortConfig,
    /// Retired setting that Herdr wrote before the workspace filter was removed.
    #[serde(rename = "agent_panel_scope")]
    _legacy_agent_panel_scope: Option<LegacyAgentPanelScopeConfig>,
    /// Agent status indicator style. Saved values are "dots" or "symbols". Default: "dots".
    pub status_indicators: StatusIndicatorStyle,
    /// Expanded sidebar row composition.
    pub sidebar: SidebarConfig,
    /// Sidebar workspace list ordering. "manual" keeps the user's drag order,
    /// "priority" bubbles attention-needing workspaces to the top. Default: "manual".
    pub workspace_sort: WorkspaceSortConfig,
    /// How priority-sorted lists (spaces, agents panel) apply reorders.
    /// "bubble" holds a row in place for `sort_motion_settle_ms`, then moves
    /// it one position per `sort_motion_step_ms` so the list never teleports
    /// under the cursor; "instant" re-sorts immediately. Default: "bubble".
    pub sort_motion: SortMotionConfig,
    /// How long a row holds its position after its sort position changes
    /// before it starts bubbling, in milliseconds. Default: 2000.
    pub sort_motion_settle_ms: u64,
    /// Interval between one-position bubble steps, in milliseconds.
    /// Default: 150.
    pub sort_motion_step_ms: u64,
    /// Step cadence across a reshuffle. "linear" spaces every step evenly;
    /// "bubble" eases in and out — slow to break away, quickest mid-flight,
    /// slowing into the final slot. Only visible on longer travels.
    /// Default: "linear".
    pub sort_motion_easing: SortMotionEasingConfig,
    /// Sidebar entry composition. "default" keeps the current layout;
    /// "editorial" right-aligns jump numbers on the name row, renders thin
    /// uppercase section headers, and dims inactive meta lines.
    pub sidebar_style: SidebarStyleConfig,
    /// Per-state color overrides for sidebar state glyphs and state text
    /// (working/idle/done/blocked/unknown). Same syntax as `accent`.
    pub state_colors: StateColorsConfig,
    /// Per-state sidebar icon glyph overrides; see `StateSymbolsConfig`.
    pub state_symbols: StateSymbolsConfig,
    /// Working-icon animation in agent rows. Saved values are "on" or "off". Default: "on".
    pub status_spinner: StatusSpinnerConfig,
    /// Milliseconds between spinner frames, clamped to 50..=2000. Default: 200.
    pub status_spinner_ms: u64,
    /// Milliseconds the `prefix+i` labels and the resize labels stay up,
    /// clamped to 500..=60000. Default: 3000.
    pub display_panes_ms: u64,
    /// Notification center position. "top-right" puts the indicator in the
    /// tab bar with the dropdown under its right edge; "bottom-right" floats
    /// the indicator in the frame's bottom-right corner with the dropdown
    /// opening above it. Default: "top-right".
    pub notification_center_position: NotificationCenterPositionConfig,
    /// Accent color for highlights, borders, and navigation UI.
    /// Accepts hex (#89b4fa), named colors (cyan, blue), or RGB (rgb(137,180,250)).
    pub accent: String,
    /// Color for `show_workspace_numbers` labels. Same syntax as `accent`.
    /// Unset uses the theme's muted number color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_number_color: Option<String>,
    /// Color for `show_agent_numbers` labels. Same syntax as `accent`.
    /// Unset uses the theme's muted number color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_number_color: Option<String>,
    /// Leader glyph(s) shown before the workspace jump number in
    /// `sidebar_style = "editorial"`, e.g. "₽" to hint the `prefix + N`
    /// chord. Rendered in `workspace_number_color`. Default empty (bare number).
    #[serde(default)]
    pub workspace_number_prefix: String,
    /// Leader glyph(s) shown before the agent jump number in
    /// `sidebar_style = "editorial"`, e.g. "₽⌥" to hint the `prefix + alt + N`
    /// chord. Rendered in `agent_number_color`. Default empty (bare number).
    #[serde(default)]
    pub agent_number_prefix: String,
    /// Override color for the focused (active) pane border. Same syntax as `accent`.
    /// Unset uses the theme accent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_border_active_color: Option<String>,
    /// Override color for unfocused (inactive) pane borders. Same syntax as `accent`.
    /// Unset uses the theme's muted border color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_border_inactive_color: Option<String>,
    /// Box-drawing weight for the focused pane border: "light", "heavy", or
    /// "double". Default: "light".
    pub pane_border_active_style: PaneBorderActiveStyleConfig,
    /// Override color for the focused pane's border title. Same syntax as `accent`.
    /// Unset follows `pane_border_active_color`, then the theme accent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_title_active_color: Option<String>,
    /// Override color for unfocused panes' border titles. Same syntax as `accent`.
    /// Unset follows `pane_border_inactive_color`, then the theme's muted color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_title_inactive_color: Option<String>,
    /// Override colour for the pane todo indicator while todos are outstanding.
    /// Same syntax as `accent`. Unset colours it by the highest outstanding
    /// priority (high red, normal yellow, low blue); an all-done indicator is
    /// always muted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_todo_color: Option<String>,
    /// Highlight pattern for the active space and agent in the sidebar, using
    /// `pane_border_active_color` and `pane_border_active_style`. Accepts
    /// "off", "above", "below", "both", "left", "right" — or a bool for
    /// backward compatibility (true = "both"). Default: off.
    pub sidebar_active_border: SidebarActiveBorderConfig,
    /// How the spaces list and agent panel show rows scrolled out of view:
    /// "both" (default), "rows" (summary rows at the edge), "fog" (a lighter
    /// background on the rows next to the edge) or "off".
    pub sidebar_overflow: SidebarOverflowConfig,
    /// How far the fog lifts the rows next to a hidden edge, in percent of the
    /// way from the background to the text colour, nearest row first. 0 to 2
    /// entries, each clamped to 0..=60; 0 (or no entry) means no fog on that
    /// row. Default: [17, 7].
    pub sidebar_fog: Vec<i64>,
    /// How much of the most urgent hidden state's colour the fog takes,
    /// 0..=100 (0: a neutral lift). Default: 70.
    pub sidebar_fog_tint: i64,
    /// What the fog does: "lift" (a lighter background, default), "dim" (the
    /// text fades toward the background) or "both". Unknown values use "lift".
    pub sidebar_fog_style: String,
    /// How far the "dim" and "both" fog styles move the text of the fogged
    /// rows toward the fog base, in percent, nearest row first. 0 to 2
    /// entries, each clamped to 0..=95; 0 (or no entry) means no fade on that
    /// row. Default: [85, 55].
    pub sidebar_fade: Vec<i64>,
    /// Default background for the focused pane's cells (tmux
    /// `window-active-style` bg). Same syntax as `accent`. Only cells without
    /// an explicit app-painted background are tinted. Unset keeps the
    /// terminal default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_active_bg: Option<String>,
    /// Default background for unfocused panes' cells (tmux `window-style`
    /// bg). Same syntax and semantics as `pane_active_bg`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_inactive_bg: Option<String>,
    /// Dim unfocused pane content in all modes, not only while a herdr mode
    /// (prefix/navigate) is active. Default: false.
    pub dim_inactive_panes: bool,
    /// How far an unfocused pane's text colour moves toward the colour behind
    /// it, in percent (0..=90, clamped with a diagnostic). A real colour change
    /// that stays lighter than the SGR faint `dim_inactive_panes` and prefix
    /// mode use. 0 is off. Default: 0.
    pub inactive_pane_dim: i64,
    /// Optional visual toast notifications for background workspace events.
    pub toast: ToastConfig,
    /// Play sounds when agents change state in background workspaces.
    pub sound: SoundConfig,
}

/// Cursor shape (DECSCUSR) used for the forced IME anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImeCursorShape {
    Block,
    #[default]
    SteadyBlock,
    Underline,
    SteadyUnderline,
    Bar,
    SteadyBar,
}

impl ImeCursorShape {
    /// Convert to DECSCUSR parameter (1–6).
    pub fn to_decscusr(self) -> u8 {
        match self {
            Self::Block => 1,
            Self::SteadyBlock => 2,
            Self::Underline => 3,
            Self::SteadyUnderline => 4,
            Self::Bar => 5,
            Self::SteadyBar => 6,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Windows: allow ordinary same-account clients to control an elevated server. Default: false.
    #[cfg(windows)]
    pub allow_unelevated_clients: bool,
    /// Virtual terminal width used when no client is attached. Default: 120.
    pub headless_cols: u16,
    /// Virtual terminal height used when no client is attached. Default: 40.
    pub headless_rows: u16,
    /// Use the last attached client's size, not the headless size, while no
    /// client is attached, and keep it across restarts. Default: true.
    pub remember_client_size: bool,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct AdvancedConfig {
    /// Maximum scrollback buffer size in bytes retained per pane terminal. Default: 10000000.
    #[serde(alias = "scrollback_lines")]
    pub scrollback_limit_bytes: usize,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct RemoteConfig {
    /// Add keepalive fallbacks and private connection reuse for `herdr --remote`.
    /// Set false to run plain ssh unchanged. Default: true.
    pub manage_ssh_config: bool,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            manage_ssh_config: true,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AgentsConfig {
    pub codex: CodexAgentConfig,
}

/// How herdr launches Codex panes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CodexAgentConfig {
    /// Launch and resume Codex panes on the shared Codex app-server daemon
    /// (`--remote unix://<app_server_socket>` and `-C <pane cwd>`). Default: false.
    pub app_server: bool,
    /// The daemon's control socket. `~` expands to the home directory.
    /// Default: `~/.codex/app-server-control/app-server-control.sock`.
    pub app_server_socket: String,
    /// Name each Codex pane's daemon thread after the pane's agent name.
    /// Needs `app_server`. Default: false.
    pub name_threads: bool,
}

impl Default for CodexAgentConfig {
    fn default() -> Self {
        Self {
            app_server: false,
            app_server_socket: DEFAULT_CODEX_APP_SERVER_SOCKET.to_string(),
            name_threads: false,
        }
    }
}

pub const DEFAULT_CODEX_APP_SERVER_SOCKET: &str =
    "~/.codex/app-server-control/app-server-control.sock";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ExperimentalConfig {
    /// Allow launching herdr inside an existing herdr pane. Default: false.
    pub allow_nested: bool,
    /// Deprecated compatibility key for `terminal.kitty_graphics`.
    pub kitty_graphics: Option<bool>,
    /// Persist pane screen history to session-history.json. Default: false.
    pub pane_history: bool,
    /// Expose the focused pane's cursor anchor to the outer terminal even when
    /// the pane requested `?25l`, so macOS native input methods keep tracking
    /// the candidate window when TUIs paint their own cursor (Claude Code, pi,
    /// codex, etc.). Default: false.
    ///
    /// When the pane reports no cursor position, falls back to the pane's
    /// top-left so a stable IME anchor is always available.
    ///
    /// Trade-off when enabled: an extra hardware cursor will be visible in the
    /// outer terminal for apps that hide the cursor without painting a
    /// replacement (vim normal mode, etc.). See #149.
    pub reveal_hidden_cursor_for_cjk_ime: bool,
    /// Restrict `reveal_hidden_cursor_for_cjk_ime` to focused panes whose
    /// detected agent matches one of these names (case-insensitive). Empty
    /// list means apply to any focused pane. Unknown agent names are ignored;
    /// if the list contains no valid names, the reveal does not apply.
    /// Accepted names: pi, claude, codex, gemini, cursor, devin, cline,
    /// opencode, copilot, kimi, kiro, droid, amp, grok, hermes, kilo,
    /// qodercli, qoder, qwen, qwen-code, letta, letta-code, maki.
    /// Default: empty.
    pub cjk_ime_agents: Vec<String>,
    /// Cursor shape rendered for the IME anchor when
    /// `reveal_hidden_cursor_for_cjk_ime` is enabled. Default: "steady_block".
    pub cjk_ime_cursor_shape: ImeCursorShape,
    /// While prefix mode is active, temporarily switch the host input source
    /// to an ASCII-capable mode so prefix commands are read as ASCII even when
    /// an IME is active, then restore the previous input source when prefix
    /// mode exits. On macOS this selects the ASCII-capable keyboard layout; on
    /// Windows it switches the IME to English (ASCII) input. Windows support is
    /// currently limited to the Korean IME; with an IME for any other language,
    /// the input source is left unchanged. macOS and Windows only; a no-op
    /// elsewhere and a best-effort no-op if the switch fails.
    /// Default: false.
    pub switch_ascii_input_source_in_prefix: bool,
}

impl Default for KeysConfig {
    fn default() -> Self {
        Self {
            prefix: BindingConfig::one("ctrl+b"),
            help: BindingConfig::one("prefix+?"),
            settings: BindingConfig::one("prefix+s"),
            new_workspace: BindingConfig::one("prefix+shift+n"),
            new_worktree: BindingConfig::one("prefix+shift+g"),
            open_worktree: BindingConfig::empty(),
            remove_worktree: BindingConfig::empty(),
            rename_workspace: BindingConfig::one("prefix+shift+w"),
            toggle_pin_workspace: BindingConfig::empty(),
            toggle_sync_panes: BindingConfig::one("prefix+shift+s"),
            toggle_pin_agent: BindingConfig::empty(),
            close_workspace: BindingConfig::one("prefix+shift+d"),
            workspace_picker: BindingConfig::one("prefix+w"),
            goto: BindingConfig::one("prefix+g"),
            navigate_workspace_up: BindingConfig::one("up"),
            navigate_workspace_down: BindingConfig::one("down"),
            navigate_pane_left: BindingConfig::one("h"),
            navigate_pane_down: BindingConfig::one("j"),
            navigate_pane_up: BindingConfig::one("k"),
            navigate_pane_right: BindingConfig::one("l"),
            detach: BindingConfig::one("prefix+q"),
            reload_config: BindingConfig::one("prefix+shift+r"),
            open_notification_target: BindingConfig::one("prefix+o"),
            open_notification_center: BindingConfig::one("prefix+ctrl+n"),
            open_pane_todos: BindingConfig::one("prefix+ctrl+t"),
            add_pane_todo: BindingConfig::empty(),
            open_todo_board: BindingConfig::empty(),
            display_panes: BindingConfig::one("prefix+i"),
            previous_workspace: BindingConfig::empty(),
            next_workspace: BindingConfig::empty(),
            previous_agent: BindingConfig::empty(),
            next_agent: BindingConfig::empty(),
            focus_agent: BindingConfig::empty(),
            remote_image_paste: "ctrl+v".into(),
            new_tab: BindingConfig::one("prefix+c"),
            rename_tab: BindingConfig::one("prefix+shift+t"),
            previous_tab: BindingConfig::one("prefix+p"),
            next_tab: BindingConfig::one("prefix+n"),
            move_tab_previous: BindingConfig::empty(),
            move_tab_next: BindingConfig::empty(),
            switch_tab: BindingConfig::one("prefix+1..9"),
            switch_workspace: BindingConfig::empty(),
            close_tab: BindingConfig::one("prefix+shift+x"),
            rename_pane: BindingConfig::one("prefix+shift+p"),
            break_pane: BindingConfig::one("prefix+!"),
            move_pane_to_tab: BindingConfig::one("prefix+m"),
            move_pane_next_tab: BindingConfig::one("prefix+>"),
            move_pane_prev_tab: BindingConfig::one("prefix+<"),
            edit_scrollback: BindingConfig::one("prefix+e"),
            clear_pane: BindingConfig::default(),
            clear_scrollback: BindingConfig::empty(),
            copy_mode: BindingConfig::one("prefix+["),
            copy_mode_page_up: BindingConfig::one("prefix+pageup"),
            copy_mode_half_page_up: BindingConfig::one("prefix+ctrl+u"),
            copy_mode_line_up: BindingConfig::one("prefix+ctrl+k"),
            copy_mode_page_down: BindingConfig::one("prefix+pagedown"),
            copy_mode_half_page_down: BindingConfig::one("prefix+ctrl+d"),
            copy_mode_line_down: BindingConfig::one("prefix+ctrl+j"),
            focus_pane_left: BindingConfig::one("prefix+h"),
            focus_pane_down: BindingConfig::one("prefix+j"),
            focus_pane_up: BindingConfig::one("prefix+k"),
            focus_pane_right: BindingConfig::one("prefix+l"),
            swap_pane_left: BindingConfig::one("prefix+shift+h"),
            swap_pane_down: BindingConfig::one("prefix+shift+j"),
            swap_pane_up: BindingConfig::one("prefix+shift+k"),
            swap_pane_right: BindingConfig::one("prefix+shift+l"),
            cycle_pane_next: BindingConfig::one("prefix+tab"),
            cycle_pane_previous: BindingConfig::one("prefix+shift+tab"),
            last_pane: BindingConfig::empty(),
            split_vertical: BindingConfig::one("prefix+v"),
            split_horizontal: BindingConfig::one("prefix+minus"),
            close_pane: BindingConfig::one("prefix+x"),
            respawn_pane: BindingConfig::one("prefix+ctrl+x"),
            zoom: BindingConfig::one("prefix+z"),
            resize_mode: BindingConfig::one("prefix+r"),
            resize_pane_left: BindingConfig::empty(),
            resize_pane_down: BindingConfig::empty(),
            resize_pane_up: BindingConfig::empty(),
            resize_pane_right: BindingConfig::empty(),
            balance_panes: BindingConfig::one("prefix+="),
            next_layout: BindingConfig::one("prefix+space"),
            toggle_sidebar: BindingConfig::one("prefix+b"),
            indexed: IndexedKeysConfig::default(),
            command: Vec::new(),
            user_fields: BTreeSet::new(),
        }
    }
}

impl Default for WorktreesConfig {
    fn default() -> Self {
        Self {
            directory: "~/.herdr/worktrees".into(),
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            sidebar_width: 26,
            sidebar_min_width: 18,
            sidebar_max_width: 36,
            sidebar_start_collapsed: false,
            sidebar_collapsed_mode: SidebarCollapsedModeConfig::Compact,
            mobile_width_threshold: DEFAULT_MOBILE_WIDTH_THRESHOLD,
            mouse_capture: true,
            copy_on_select: true,
            host_cursor: HostCursorModeConfig::Auto,
            right_click_passthrough_modifier: RightClickPassthroughModifierConfig::default(),
            redraw_on_focus_gained: true,
            mouse_scroll_lines: None,
            confirm_close: true,
            prompt_new_tab_name: true,
            prompt_new_workspace_name: false,
            pane_borders: PaneBordersConfig::Auto,
            pane_outer_borders: true,
            pane_scrollbars: true,
            pane_gaps: true,
            show_agent_labels_on_pane_borders: false,
            show_pane_todo_indicator: true,
            hide_tab_bar_when_single_tab: false,
            tab_bar_position: TabBarPositionConfig::Top,
            tab_bar_right: Vec::new(),
            tab_bar_right_separator: " ".into(),
            window_title: super::window_title::default_window_title(),
            show_workspace_numbers: false,
            show_agent_numbers: false,
            show_host: true,
            agent_panel_sort: AgentPanelSortConfig::Spaces,
            _legacy_agent_panel_scope: None,
            status_indicators: StatusIndicatorStyle::Dots,
            sidebar: SidebarConfig::default(),
            workspace_sort: WorkspaceSortConfig::Manual,
            sort_motion: SortMotionConfig::Bubble,
            sort_motion_settle_ms: 2000,
            sort_motion_step_ms: 150,
            sort_motion_easing: SortMotionEasingConfig::Linear,
            sidebar_style: SidebarStyleConfig::Default,
            state_colors: StateColorsConfig::default(),
            state_symbols: StateSymbolsConfig::default(),
            status_spinner: StatusSpinnerConfig::default(),
            status_spinner_ms: DEFAULT_STATUS_SPINNER_MS,
            display_panes_ms: DEFAULT_DISPLAY_PANES_MS,
            notification_center_position: NotificationCenterPositionConfig::TopRight,
            accent: "cyan".into(),
            workspace_number_color: None,
            agent_number_color: None,
            workspace_number_prefix: String::new(),
            agent_number_prefix: String::new(),
            pane_border_active_color: None,
            pane_border_inactive_color: None,
            pane_border_active_style: PaneBorderActiveStyleConfig::Light,
            pane_title_active_color: None,
            pane_title_inactive_color: None,
            pane_todo_color: None,
            sidebar_active_border: SidebarActiveBorderConfig::Off,
            sidebar_overflow: SidebarOverflowConfig::default(),
            sidebar_fog: vec![17, 7],
            sidebar_fog_tint: 70,
            sidebar_fog_style: "lift".to_string(),
            sidebar_fade: vec![85, 55],
            pane_active_bg: None,
            pane_inactive_bg: None,
            dim_inactive_panes: false,
            inactive_pane_dim: 0,
            toast: ToastConfig::default(),
            sound: SoundConfig::default(),
        }
    }
}

impl UiConfig {
    pub fn mouse_scroll_lines(&self) -> usize {
        self.mouse_scroll_lines
            .map(NonZeroUsize::get)
            .unwrap_or(DEFAULT_MOUSE_SCROLL_LINES)
    }

    pub fn right_click_passthrough_modifiers(&self) -> Option<KeyModifiers> {
        self.right_click_passthrough_modifier.modifiers()
    }
}

impl Default for ToastConfig {
    fn default() -> Self {
        Self {
            delivery: ToastDelivery::Off,
            delay_seconds: 1,
            herdr: HerdrToastConfig::default(),
            clipboard: ClipboardToastConfig::default(),
        }
    }
}

impl Default for HerdrToastConfig {
    fn default() -> Self {
        Self {
            position: ToastHerdrPosition::BottomRight,
            size: ToastHerdrSize::Auto,
            needs_attention_seconds: 8,
            finished_seconds: 5,
            update_seconds: 3,
            pane_feedback: ToastPaneFeedback::Corner,
        }
    }
}

impl Default for ClipboardToastConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            position: ToastClipboardPosition::BottomCenter,
        }
    }
}

impl<'de> Deserialize<'de> for ToastConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct RawToastConfig {
            delivery: Option<ToastDelivery>,
            enabled: Option<bool>,
            delay_seconds: Option<u64>,
            herdr: HerdrToastConfig,
            clipboard: ClipboardToastConfig,
        }

        let raw = RawToastConfig::deserialize(deserializer)?;
        let legacy_delivery = match raw.enabled {
            Some(true) => ToastDelivery::Herdr,
            Some(false) | None => ToastDelivery::Off,
        };
        let delivery = raw.delivery.unwrap_or(legacy_delivery);
        let default = Self::default();
        let delay_seconds = raw.delay_seconds.unwrap_or(default.delay_seconds);
        if delay_seconds > MAX_TOAST_DELAY_SECONDS {
            return Err(de::Error::custom(format!(
                "ui.toast.delay_seconds must be between 0 and {MAX_TOAST_DELAY_SECONDS}"
            )));
        }
        Ok(Self {
            delivery,
            delay_seconds,
            herdr: raw.herdr,
            clipboard: raw.clipboard,
        })
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            #[cfg(windows)]
            allow_unelevated_clients: false,
            headless_cols: crate::config::DEFAULT_HEADLESS_COLS,
            headless_rows: crate::config::DEFAULT_HEADLESS_ROWS,
            remember_client_size: true,
        }
    }
}

impl Default for AdvancedConfig {
    fn default() -> Self {
        Self {
            scrollback_limit_bytes: DEFAULT_SCROLLBACK_LIMIT_BYTES,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_config_defaults_and_parses() {
        let default_config = Config::default();
        assert_eq!(default_config.update.channel, default_update_channel());
        assert!(default_config.update.version_check);
        assert!(default_config.update.manifest_check);

        let toml = r#"
[update]
channel = "preview"
version_check = false
manifest_check = false
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.update.channel, UpdateChannelConfig::Preview);
        assert_eq!(config.update.channel.as_str(), "preview");
        assert!(!config.update.version_check);
        assert!(!config.update.manifest_check);
    }

    #[test]
    fn ui_show_host_defaults_true_and_parses_override() {
        assert!(Config::default().ui.show_host);

        // A `[ui]` table without the key inherits the default.
        let inherited: Config = toml::from_str("[ui]\nsidebar_width = 30\n").unwrap();
        assert!(inherited.ui.show_host);

        // An explicit override wins.
        let overridden: Config = toml::from_str("[ui]\nshow_host = false\n").unwrap();
        assert!(!overridden.ui.show_host);
    }
    #[test]
    fn update_channel_default_follows_windows_build_identity() {
        assert_eq!(
            default_update_channel_for_build(true, true),
            UpdateChannelConfig::Preview
        );
        assert_eq!(
            default_update_channel_for_build(true, false),
            UpdateChannelConfig::Stable
        );
        assert_eq!(
            default_update_channel_for_build(false, true),
            UpdateChannelConfig::Stable
        );
    }

    #[test]
    fn missing_update_channel_uses_build_default() {
        let empty: Config = toml::from_str("").unwrap();
        let without_update_channel: Config =
            toml::from_str("[update]\nversion_check = false").unwrap();

        assert_eq!(Config::default().update.channel, default_update_channel());
        assert_eq!(empty.update.channel, default_update_channel());
        assert_eq!(
            without_update_channel.update.channel,
            default_update_channel()
        );
    }

    #[test]
    fn terminal_default_shell_defaults_empty_and_parses() {
        let default_config = Config::default();
        assert!(default_config.terminal.default_shell.is_empty());
        assert_eq!(default_config.terminal.shell_mode, ShellModeConfig::Auto);

        let toml = r#"
[terminal]
default_shell = "nu"
shell_mode = "non_login"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.terminal.default_shell, "nu");
        assert_eq!(config.terminal.shell_mode, ShellModeConfig::NonLogin);
    }

    #[test]
    fn terminal_new_cwd_defaults_follow_and_parses() {
        let default_config = Config::default();
        assert_eq!(
            default_config.terminal.new_cwd,
            NewTerminalCwdConfig::Follow
        );

        let config: Config = toml::from_str(
            r#"
[terminal]
new_cwd = "home"
"#,
        )
        .unwrap();
        assert_eq!(config.terminal.new_cwd, NewTerminalCwdConfig::Home);

        let config: Config = toml::from_str(
            r#"
[terminal]
new_cwd = "~/Projects"
"#,
        )
        .unwrap();
        assert_eq!(
            config.terminal.new_cwd,
            NewTerminalCwdConfig::Path("~/Projects".into())
        );
    }

    #[test]
    fn resume_agents_on_restore_defaults_on_and_parses() {
        let default_config = Config::default();
        assert!(default_config.session.resume_agents_on_restore);
        assert_eq!(default_config.session.startup_per_agent_delay_ms, 100);

        let toml = r#"
[session]
resume_agents_on_restore = false
startup_per_agent_delay_ms = 0
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(!config.session.resume_agents_on_restore);
        assert_eq!(config.session.startup_per_agent_delay_ms, 0);
    }

    #[test]
    fn agent_panel_sort_config_parses_alias_and_defaults() {
        assert_eq!(
            Config::default().ui.agent_panel_sort,
            AgentPanelSortConfig::Spaces
        );

        let toml = r#"
[ui]
agent_panel_sort = "priority"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.agent_panel_sort, AgentPanelSortConfig::Priority);

        let toml = r#"
[ui]
agent_panel_sort = "workspaces"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.agent_panel_sort, AgentPanelSortConfig::Spaces);

        let toml = r#"
[ui]
agent_panel_scope = "current"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.agent_panel_sort, AgentPanelSortConfig::Spaces);
    }

    #[test]
    fn status_indicator_style_defaults_to_dots_and_parses_symbols() {
        assert_eq!(
            Config::default().ui.status_indicators,
            StatusIndicatorStyle::Dots
        );

        let config: Config = toml::from_str(
            r#"
[ui]
status_indicators = "symbols"
"#,
        )
        .unwrap();
        assert_eq!(config.ui.status_indicators, StatusIndicatorStyle::Symbols);
    }

    #[test]
    fn pane_borders_legacy_booleans_map_to_modes() {
        let enabled: Config = toml::from_str("[ui]\npane_borders = true").unwrap();
        assert_eq!(enabled.ui.pane_borders, PaneBordersConfig::Auto);

        let disabled: Config = toml::from_str("[ui]\npane_borders = false").unwrap();
        assert_eq!(disabled.ui.pane_borders, PaneBordersConfig::Off);

        let auto: Config = toml::from_str("[ui]\npane_borders = \"auto\"").unwrap();
        assert_eq!(auto.ui.pane_borders, PaneBordersConfig::Auto);

        let off: Config = toml::from_str("[ui]\npane_borders = \"off\"").unwrap();
        assert_eq!(off.ui.pane_borders, PaneBordersConfig::Off);

        let unknown = toml::from_str::<Config>("[ui]\npane_borders = \"framed\"")
            .unwrap_err()
            .to_string();
        assert!(unknown.contains("\"auto\", \"always\", \"off\", or a legacy boolean"));

        let wrong_type = toml::from_str::<Config>("[ui]\npane_borders = 3")
            .unwrap_err()
            .to_string();
        assert!(wrong_type.contains("\"auto\", \"always\", \"off\", or a legacy boolean"));
    }

    #[test]
    fn pane_border_and_title_config_parses_and_defaults() {
        let defaults = Config::default();
        assert_eq!(
            defaults.ui.pane_border_active_style,
            PaneBorderActiveStyleConfig::Light
        );
        assert_eq!(defaults.ui.pane_border_active_color, None);
        assert_eq!(defaults.ui.pane_border_inactive_color, None);
        assert_eq!(defaults.ui.pane_title_active_color, None);
        assert_eq!(defaults.ui.pane_title_inactive_color, None);
        assert_eq!(
            defaults.ui.sidebar_active_border,
            SidebarActiveBorderConfig::Off
        );
        assert_eq!(defaults.ui.pane_active_bg, None);
        assert_eq!(defaults.ui.pane_inactive_bg, None);
        assert!(!defaults.ui.dim_inactive_panes);

        let toml = r##"
[ui]
pane_border_active_style = "heavy"
pane_border_active_color = "#d78700"
pane_border_inactive_color = "#4a4a4a"
pane_title_active_color = "#ffd700"
pane_title_inactive_color = "#7a7a7a"
sidebar_active_border = true
pane_active_bg = "#000000"
pane_inactive_bg = "#0c0c0c"
dim_inactive_panes = true
"##;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(
            config.ui.pane_border_active_style,
            PaneBorderActiveStyleConfig::Heavy
        );
        assert_eq!(
            config.ui.pane_border_active_color.as_deref(),
            Some("#d78700")
        );
        assert_eq!(
            config.ui.pane_border_inactive_color.as_deref(),
            Some("#4a4a4a")
        );
        assert_eq!(
            config.ui.pane_title_active_color.as_deref(),
            Some("#ffd700")
        );
        assert_eq!(
            config.ui.pane_title_inactive_color.as_deref(),
            Some("#7a7a7a")
        );
        // legacy bool maps to both/off
        assert_eq!(
            config.ui.sidebar_active_border,
            SidebarActiveBorderConfig::Both
        );
        assert_eq!(config.ui.pane_active_bg.as_deref(), Some("#000000"));
        assert_eq!(config.ui.pane_inactive_bg.as_deref(), Some("#0c0c0c"));
        assert!(config.ui.dim_inactive_panes);
        let config: Config = toml::from_str("[ui]\nsidebar_active_border = false").unwrap();
        assert_eq!(
            config.ui.sidebar_active_border,
            SidebarActiveBorderConfig::Off
        );

        for (value, expected) in [
            ("off", SidebarActiveBorderConfig::Off),
            ("above", SidebarActiveBorderConfig::Above),
            ("below", SidebarActiveBorderConfig::Below),
            ("both", SidebarActiveBorderConfig::Both),
            ("left", SidebarActiveBorderConfig::Left),
            ("right", SidebarActiveBorderConfig::Right),
        ] {
            let toml = format!("[ui]\nsidebar_active_border = \"{value}\"");
            let config: Config = toml::from_str(&toml).unwrap();
            assert_eq!(config.ui.sidebar_active_border, expected, "{value}");
        }
        assert!(toml::from_str::<Config>("[ui]\nsidebar_active_border = \"under\"").is_err());

        let toml = r#"
[ui]
pane_border_active_style = "double"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(
            config.ui.pane_border_active_style,
            PaneBorderActiveStyleConfig::Double
        );

        let toml = r#"
[ui]
pane_border_active_style = "thick"
"#;
        assert!(toml::from_str::<Config>(toml).is_err());
    }

    #[test]
    fn workspace_sort_config_parses_and_defaults() {
        assert_eq!(
            Config::default().ui.workspace_sort,
            WorkspaceSortConfig::Manual
        );

        let toml = r#"
[ui]
workspace_sort = "priority"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.workspace_sort, WorkspaceSortConfig::Priority);

        let toml = r#"
[ui]
workspace_sort = "manual"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.workspace_sort, WorkspaceSortConfig::Manual);

        let toml = r#"
[ui]
agent_panel_scope = "current"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.workspace_sort, WorkspaceSortConfig::Manual);
    }

    #[test]
    fn number_prefixes_parse_and_default() {
        let defaults = Config::default();
        assert_eq!(defaults.ui.workspace_number_prefix, "");
        assert_eq!(defaults.ui.agent_number_prefix, "");

        let toml = r#"
[ui]
workspace_number_prefix = "₽"
agent_number_prefix = "₽⌥"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.workspace_number_prefix, "₽");
        assert_eq!(config.ui.agent_number_prefix, "₽⌥");
    }

    #[test]
    fn sidebar_style_and_state_colors_parse_and_default() {
        let defaults = Config::default();
        assert_eq!(defaults.ui.sidebar_style, SidebarStyleConfig::Default);
        assert_eq!(defaults.ui.state_colors, StateColorsConfig::default());

        let toml = r##"
[ui]
sidebar_style = "editorial"
[ui.state_colors]
working = "#ffc832"
idle = "#4ade80"
"##;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.sidebar_style, SidebarStyleConfig::Editorial);
        assert_eq!(config.ui.state_colors.working.as_deref(), Some("#ffc832"));
        assert_eq!(config.ui.state_colors.idle.as_deref(), Some("#4ade80"));
        assert_eq!(config.ui.state_colors.done, None);
        assert_eq!(config.ui.state_colors.blocked, None);
    }

    #[test]
    fn display_panes_ms_defaults_to_three_seconds_and_parses() {
        assert_eq!(Config::default().ui.display_panes_ms, 3000);
        assert_eq!(DEFAULT_DISPLAY_PANES_MS, 3000);
        let config: Config = toml::from_str("[ui]\ndisplay_panes_ms = 5000\n").unwrap();
        assert_eq!(config.ui.display_panes_ms, 5000);
        assert!(config.collect_diagnostics().is_empty());
    }

    #[test]
    fn an_out_of_range_display_panes_ms_is_reported_and_clamped() {
        for ms in [0, 499, 60_001] {
            let config: Config =
                toml::from_str(&format!("[ui]\ndisplay_panes_ms = {ms}\n")).unwrap();
            let diagnostics = config.collect_diagnostics();
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d.contains("ui.display_panes_ms")),
                "{ms}: {diagnostics:?}"
            );
        }
        assert_eq!(clamp_display_panes_ms(0), MIN_DISPLAY_PANES_MS);
        assert_eq!(clamp_display_panes_ms(u64::MAX), MAX_DISPLAY_PANES_MS);
        assert_eq!(clamp_display_panes_ms(4000), 4000);
    }

    #[test]
    fn status_spinner_parses_with_its_beta_alias_and_defaults_on() {
        let defaults = Config::default();
        assert_eq!(defaults.ui.status_spinner, StatusSpinnerConfig::On);
        assert_eq!(defaults.ui.status_spinner_ms, DEFAULT_STATUS_SPINNER_MS);

        let config: Config =
            toml::from_str("[ui]\nstatus_spinner = \"off\"\nstatus_spinner_ms = 125\n").unwrap();
        assert_eq!(config.ui.status_spinner, StatusSpinnerConfig::Off);
        assert_eq!(config.ui.status_spinner_ms, 125);

        // The two betas that shipped the title-clocked spinner wrote "agent".
        let config: Config = toml::from_str("[ui]\nstatus_spinner = \"agent\"\n").unwrap();
        assert_eq!(config.ui.status_spinner, StatusSpinnerConfig::On);
    }

    #[test]
    fn state_symbols_parse_default_and_reject_multi_cell_glyphs() {
        assert_eq!(
            Config::default().ui.state_symbols,
            StateSymbolsConfig::default()
        );

        let toml = r##"
[ui.state_symbols]
done = "●"
idle = "✔"
background = "⊙"
background_alt = "⊚"
working = "⠋⠙"
blocked = ""
unknown = "日"
"##;
        let config: Config = toml::from_str(toml).unwrap();
        let symbols = &config.ui.state_symbols;
        assert_eq!(StateSymbolsConfig::valid(&symbols.done), Some("●"));
        assert_eq!(StateSymbolsConfig::valid(&symbols.idle), Some("✔"));
        assert_eq!(StateSymbolsConfig::valid(&symbols.background), Some("⊙"));
        assert_eq!(
            StateSymbolsConfig::valid(&symbols.background_alt),
            Some("⊚")
        );
        // Two cells, zero cells, and a double-width CJK glyph would all shift
        // the icon column, so they are reported and dropped.
        assert_eq!(StateSymbolsConfig::valid(&symbols.working), None);
        assert_eq!(StateSymbolsConfig::valid(&symbols.blocked), None);
        assert_eq!(StateSymbolsConfig::valid(&symbols.unknown), None);
        assert_eq!(
            symbols.diagnostics(),
            vec![
                "ui.state_symbols.working = \"⠋⠙\" must be exactly one terminal cell wide; ignoring",
                "ui.state_symbols.blocked = \"\" must be exactly one terminal cell wide; ignoring",
                "ui.state_symbols.unknown = \"日\" must be exactly one terminal cell wide; ignoring",
            ]
        );
        assert!(config
            .collect_diagnostics()
            .iter()
            .any(|diag| diag.starts_with("ui.state_symbols.working")));
    }

    #[test]
    fn sort_motion_config_parses_and_defaults() {
        let defaults = Config::default();
        assert_eq!(defaults.ui.sort_motion, SortMotionConfig::Bubble);
        assert_eq!(defaults.ui.sort_motion_settle_ms, 2000);
        assert_eq!(defaults.ui.sort_motion_step_ms, 150);

        let toml = r#"
[ui]
sort_motion = "instant"
sort_motion_settle_ms = 500
sort_motion_step_ms = 80
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.sort_motion, SortMotionConfig::Instant);
        assert_eq!(config.ui.sort_motion_settle_ms, 500);
        assert_eq!(config.ui.sort_motion_step_ms, 80);

        let toml = r#"
[ui]
sort_motion = "bubble"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.sort_motion, SortMotionConfig::Bubble);
        assert_eq!(config.ui.sort_motion_settle_ms, 2000);
    }

    #[test]
    fn sort_motion_easing_parses_and_defaults() {
        assert_eq!(
            Config::default().ui.sort_motion_easing,
            SortMotionEasingConfig::Linear
        );

        let toml = r#"
[ui]
sort_motion_easing = "bubble"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.sort_motion_easing, SortMotionEasingConfig::Bubble);

        let toml = r#"
[ui]
sort_motion_easing = "linear"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.sort_motion_easing, SortMotionEasingConfig::Linear);
    }

    #[test]
    fn notification_center_position_config_parses_and_defaults() {
        assert_eq!(
            Config::default().ui.notification_center_position,
            NotificationCenterPositionConfig::TopRight
        );

        let toml = r#"
[ui]
notification_center_position = "bottom-right"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(
            config.ui.notification_center_position,
            NotificationCenterPositionConfig::BottomRight
        );

        let toml = r#"
[ui]
notification_center_position = "top-right"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(
            config.ui.notification_center_position,
            NotificationCenterPositionConfig::TopRight
        );
    }

    #[test]
    fn agent_numbers_config_defaults_and_parse() {
        let default_config = Config::default();
        assert!(!default_config.ui.show_agent_numbers);
        assert!(default_config.ui.agent_number_color.is_none());

        let toml = r##"
[ui]
show_agent_numbers = true
agent_number_color = "#a54242"
"##;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.ui.show_agent_numbers);
        assert_eq!(config.ui.agent_number_color.as_deref(), Some("#a54242"));
    }

    #[test]
    fn pane_appearance_defaults_and_parse() {
        let default_config = Config::default();
        assert_eq!(default_config.ui.pane_borders, PaneBordersConfig::Auto);
        assert!(default_config.ui.pane_outer_borders);
        assert!(default_config.ui.pane_scrollbars);
        assert!(default_config.ui.pane_gaps);
        assert!(!default_config.ui.show_agent_labels_on_pane_borders);
        assert!(!default_config.ui.hide_tab_bar_when_single_tab);
        assert_eq!(
            default_config.ui.tab_bar_position,
            TabBarPositionConfig::Top
        );
        assert!(default_config.ui.tab_bar_right.is_empty());
        assert_eq!(default_config.ui.tab_bar_right_separator, " ");

        let toml = r#"
[ui]
pane_borders = "always"
pane_outer_borders = false
pane_scrollbars = false
pane_gaps = true
show_agent_labels_on_pane_borders = true
hide_tab_bar_when_single_tab = true
tab_bar_position = "bottom"
tab_bar_right = [
  { type = "zoom" },
  { type = "hostname" },
  { type = "datetime", format = "%H:%M" },
  { type = "text", text = "prod" },
  { type = "command", command = "status.sh", interval_seconds = 10, timeout_seconds = 3 },
]
tab_bar_right_separator = " · "
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.pane_borders, PaneBordersConfig::Always);
        assert!(!config.ui.pane_outer_borders);
        assert!(!config.ui.pane_scrollbars);
        assert!(config.ui.pane_gaps);
        assert!(config.ui.show_agent_labels_on_pane_borders);
        assert!(config.ui.hide_tab_bar_when_single_tab);
        assert_eq!(config.ui.tab_bar_position, TabBarPositionConfig::Bottom);
        assert_eq!(config.ui.tab_bar_right.len(), 5);
        assert!(matches!(
            config.ui.tab_bar_right[1],
            TabBarRightEntryConfig::Hostname
        ));
        assert_eq!(config.ui.tab_bar_right_separator, " · ");
    }

    #[test]
    fn pane_todo_indicator_config_parses_and_defaults() {
        let defaults = Config::default();
        assert!(defaults.ui.show_pane_todo_indicator);
        assert_eq!(defaults.ui.pane_todo_color, None);

        let toml = r##"
[ui]
show_pane_todo_indicator = false
pane_todo_color = "#f38ba8"
"##;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(!config.ui.show_pane_todo_indicator);
        assert_eq!(config.ui.pane_todo_color.as_deref(), Some("#f38ba8"));
    }

    #[test]
    fn worktrees_directory_defaults_and_parses() {
        let default_config = Config::default();
        assert_eq!(default_config.worktrees.directory, "~/.herdr/worktrees");

        let toml = r#"
[worktrees]
directory = "~/Projects/herdr-worktrees"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.worktrees.directory, "~/Projects/herdr-worktrees");
    }

    #[test]
    fn prompt_new_tab_name_defaults_on_and_parses() {
        let default_config = Config::default();
        assert!(default_config.ui.prompt_new_tab_name);

        let toml = r#"
[ui]
prompt_new_tab_name = false
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(!config.ui.prompt_new_tab_name);
    }

    #[test]
    fn prompt_new_workspace_name_defaults_off_and_parses() {
        let default_config = Config::default();
        assert!(!default_config.ui.prompt_new_workspace_name);

        let toml = r#"
[ui]
prompt_new_workspace_name = true
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.ui.prompt_new_workspace_name);
    }

    #[test]
    fn reveal_hidden_cursor_for_cjk_ime_default_off_and_parse() {
        let default_config = Config::default();
        assert!(!default_config.experimental.reveal_hidden_cursor_for_cjk_ime);

        let toml = r#"
[experimental]
reveal_hidden_cursor_for_cjk_ime = true
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.experimental.reveal_hidden_cursor_for_cjk_ime);
    }

    #[test]
    fn switch_ascii_input_source_in_prefix_default_off_and_parse() {
        let default_config = Config::default();
        assert!(
            !default_config
                .experimental
                .switch_ascii_input_source_in_prefix
        );

        let toml = r#"
[experimental]
switch_ascii_input_source_in_prefix = true
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.experimental.switch_ascii_input_source_in_prefix);
    }

    #[test]
    fn cjk_ime_cursor_shape_default_steady_block_and_parse() {
        let default_config = Config::default();
        assert_eq!(
            default_config.experimental.cjk_ime_cursor_shape,
            ImeCursorShape::SteadyBlock
        );

        let toml = r#"
[experimental]
cjk_ime_cursor_shape = "bar"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(
            config.experimental.cjk_ime_cursor_shape,
            ImeCursorShape::Bar
        );
    }

    #[test]
    fn cjk_ime_agents_default_empty_and_parse() {
        let default_config = Config::default();
        assert!(default_config.experimental.cjk_ime_agents.is_empty());

        let toml = r#"
[experimental]
cjk_ime_agents = ["claude", "codex"]
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(
            config.experimental.cjk_ime_agents,
            vec!["claude".to_string(), "codex".to_string()]
        );
    }

    #[test]
    fn sidebar_bounds_default_and_parse() {
        let default_config = Config::default();
        assert_eq!(default_config.ui.sidebar_min_width, 18);
        assert_eq!(default_config.ui.sidebar_max_width, 36);
        assert_eq!(
            default_config.ui.mobile_width_threshold,
            DEFAULT_MOBILE_WIDTH_THRESHOLD
        );

        let toml = r#"
[ui]
sidebar_min_width = 12
sidebar_max_width = 80
mobile_width_threshold = 96
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.sidebar_min_width, 12);
        assert_eq!(config.ui.sidebar_max_width, 80);
        assert_eq!(config.ui.mobile_width_threshold, 96);
    }

    #[test]
    fn sidebar_start_collapsed_defaults_off_and_parses_on() {
        let default_config = Config::default();
        assert!(!default_config.ui.sidebar_start_collapsed);

        let toml = r#"
[ui]
sidebar_start_collapsed = true
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.ui.sidebar_start_collapsed);
    }

    #[test]
    fn sidebar_collapsed_mode_defaults_compact_and_parses_hidden() {
        let default_config = Config::default();
        assert_eq!(
            default_config.ui.sidebar_collapsed_mode,
            SidebarCollapsedModeConfig::Compact
        );

        let toml = r#"
[ui]
sidebar_collapsed_mode = "hidden"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(
            config.ui.sidebar_collapsed_mode,
            SidebarCollapsedModeConfig::Hidden
        );
    }

    #[test]
    fn validated_sidebar_bounds_rejects_inverted() {
        assert_eq!(validated_sidebar_bounds(18, 36), Some((18, 36)));
        assert_eq!(validated_sidebar_bounds(20, 20), Some((20, 20)));
        assert_eq!(validated_sidebar_bounds(0, u16::MAX), Some((0, u16::MAX)));
        assert_eq!(validated_sidebar_bounds(50, 30), None);
        assert_eq!(validated_sidebar_bounds(u16::MAX, 0), None);
    }

    #[test]
    fn mouse_capture_default_on_and_parse() {
        let default_config = Config::default();
        assert!(default_config.ui.mouse_capture);

        let toml = r#"
[ui]
mouse_capture = false
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(!config.ui.mouse_capture);
    }

    #[test]
    fn copy_on_select_default_on_and_parse() {
        let default_config = Config::default();
        assert!(default_config.ui.copy_on_select);

        let toml = r#"
[ui]
copy_on_select = false
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(!config.ui.copy_on_select);
    }

    #[test]
    fn right_click_passthrough_modifier_defaults_off_and_parses() {
        let default_config = Config::default();
        assert_eq!(default_config.ui.right_click_passthrough_modifiers(), None);

        for value in ["", "off", "none", "disabled"] {
            let toml = format!(
                r#"
[ui]
right_click_passthrough_modifier = "{value}"
"#
            );
            let config: Config = toml::from_str(&toml).unwrap();
            assert_eq!(
                config.ui.right_click_passthrough_modifiers(),
                None,
                "value {value:?} should disable passthrough"
            );
        }

        for (value, expected) in [
            ("ctrl", KeyModifiers::CONTROL),
            ("control", KeyModifiers::CONTROL),
            ("alt", KeyModifiers::ALT),
            ("option", KeyModifiers::ALT),
            ("cmd", KeyModifiers::SUPER),
            ("command", KeyModifiers::SUPER),
            ("super", KeyModifiers::SUPER),
            ("meta", KeyModifiers::META),
            ("hyper", KeyModifiers::HYPER),
        ] {
            let toml = format!(
                r#"
[ui]
right_click_passthrough_modifier = "{value}"
"#
            );
            let config: Config = toml::from_str(&toml).unwrap();
            assert_eq!(
                config.ui.right_click_passthrough_modifiers(),
                Some(expected),
                "value {value:?} should parse"
            );
        }

        let toml = r#"
[ui]
right_click_passthrough_modifier = "cmd+alt"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(
            config.ui.right_click_passthrough_modifiers(),
            Some(KeyModifiers::SUPER | KeyModifiers::ALT)
        );
    }

    #[test]
    fn right_click_passthrough_modifier_rejects_shift() {
        for value in ["shift", "shift+ctrl", "ctrl+", "ctrl++alt", "banana"] {
            let toml = format!(
                r#"
[ui]
right_click_passthrough_modifier = "{value}"
"#
            );
            assert!(
                toml::from_str::<Config>(&toml).is_err(),
                "value {value:?} should be rejected"
            );
        }
    }

    #[test]
    fn redraw_on_focus_gained_default_on_and_parse() {
        let default_config = Config::default();
        assert!(default_config.ui.redraw_on_focus_gained);

        let toml = r#"
[ui]
redraw_on_focus_gained = false
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(!config.ui.redraw_on_focus_gained);
    }

    #[test]
    fn mouse_scroll_lines_defaults_to_three_and_parses() {
        let default_config = Config::default();
        assert_eq!(
            default_config.ui.mouse_scroll_lines(),
            DEFAULT_MOUSE_SCROLL_LINES
        );

        let toml = r#"
[ui]
mouse_scroll_lines = 1
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.mouse_scroll_lines(), 1);
    }

    #[test]
    fn mouse_scroll_lines_rejects_zero() {
        let toml = r#"
[ui]
mouse_scroll_lines = 0
"#;
        assert!(toml::from_str::<Config>(toml).is_err());
    }

    // Fork issue 129.
    #[test]
    fn pane_feedback_config_parses_and_defaults_off() {
        let config: Config = toml::from_str(
            "[ui.toast.herdr]\npane_feedback = \"pane\"\n\n[ui.toast.clipboard]\nposition = \"pane\"\n",
        )
        .unwrap();
        assert_eq!(config.ui.toast.herdr.pane_feedback, ToastPaneFeedback::Pane);
        assert_eq!(
            config.ui.toast.clipboard.position,
            ToastClipboardPosition::Pane
        );

        let defaults = Config::default();
        assert_eq!(
            defaults.ui.toast.herdr.pane_feedback,
            ToastPaneFeedback::Corner
        );
        assert_eq!(
            defaults.ui.toast.clipboard.position,
            ToastClipboardPosition::BottomCenter
        );
    }

    #[test]
    fn toast_config_parses() {
        let toml = r#"
[ui.toast]
delivery = "terminal"
delay_seconds = 2

[ui.toast.herdr]
position = "top-left"

[ui.toast.clipboard]
enabled = false
position = "top-center"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.toast.delivery, ToastDelivery::Terminal);
        assert_eq!(config.ui.toast.delay_seconds, 2);
        assert_eq!(config.ui.toast.herdr.position, ToastHerdrPosition::TopLeft);
        assert!(!config.ui.toast.clipboard.enabled);
        assert_eq!(
            config.ui.toast.clipboard.position,
            ToastClipboardPosition::TopCenter
        );
    }

    #[test]
    fn toast_config_defaults_preserve_existing_behavior_with_delay() {
        let config = Config::default();
        assert_eq!(config.ui.toast.delivery, ToastDelivery::Off);
        assert_eq!(config.ui.toast.delay_seconds, 1);
        assert_eq!(
            config.ui.toast.herdr.position,
            ToastHerdrPosition::BottomRight
        );
        assert_eq!(config.ui.toast.herdr.needs_attention_seconds, 8);
        assert_eq!(config.ui.toast.herdr.finished_seconds, 5);
        assert_eq!(config.ui.toast.herdr.update_seconds, 3);
        assert!(config.ui.toast.clipboard.enabled);
        assert_eq!(
            config.ui.toast.clipboard.position,
            ToastClipboardPosition::BottomCenter
        );
    }

    #[test]
    fn toast_config_parses_center_position_and_durations() {
        let toml = r#"
[ui.toast.herdr]
position = "center"
size = "large"
needs_attention_seconds = 4
finished_seconds = 4
update_seconds = 0
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.toast.herdr.position, ToastHerdrPosition::Center);
        assert_eq!(config.ui.toast.herdr.size, ToastHerdrSize::Large);
        assert_eq!(config.ui.toast.herdr.needs_attention_seconds, 4);
        assert_eq!(config.ui.toast.herdr.finished_seconds, 4);
        assert_eq!(config.ui.toast.herdr.update_seconds, 0);
        assert_eq!(Config::default().ui.toast.herdr.size, ToastHerdrSize::Auto);
    }

    #[test]
    fn toast_config_parses_system_delivery() {
        let toml = r#"
[ui.toast]
delivery = "system"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.toast.delivery, ToastDelivery::System);
    }

    #[test]
    fn toast_config_legacy_enabled_true_maps_to_herdr() {
        let toml = r#"
[ui.toast]
enabled = true
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.toast.delivery, ToastDelivery::Herdr);
    }

    #[test]
    fn toast_config_legacy_enabled_false_maps_to_off() {
        let toml = r#"
[ui.toast]
enabled = false
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.toast.delivery, ToastDelivery::Off);
    }

    #[test]
    fn toast_config_delivery_wins_over_legacy_enabled() {
        let toml = r#"
[ui.toast]
enabled = true
delivery = "terminal"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ui.toast.delivery, ToastDelivery::Terminal);
    }

    #[test]
    fn toast_config_rejects_unbounded_delay() {
        let toml = format!(
            r#"
[ui.toast]
delay_seconds = {}
"#,
            MAX_TOAST_DELAY_SECONDS + 1
        );

        let error = toml::from_str::<Config>(&toml).unwrap_err().to_string();

        assert!(error.contains("ui.toast.delay_seconds must be between 0 and 3600"));
    }

    #[test]
    fn missing_onboarding_shows_setup() {
        let config = Config::default();
        assert!(config.should_show_onboarding());
    }

    #[test]
    fn onboarding_false_skips_setup() {
        let config: Config = toml::from_str("onboarding = false").unwrap();
        assert!(!config.should_show_onboarding());
    }

    #[test]
    fn server_headless_size_defaults_and_parses() {
        let default_config = Config::default();
        assert_eq!(
            default_config.server.headless_cols,
            crate::config::DEFAULT_HEADLESS_COLS
        );
        assert_eq!(
            default_config.server.headless_rows,
            crate::config::DEFAULT_HEADLESS_ROWS
        );

        assert!(default_config.server.remember_client_size);

        let config: Config = toml::from_str(
            r#"[server]
headless_cols = 160
headless_rows = 50
remember_client_size = false
"#,
        )
        .unwrap();
        assert_eq!(config.server.headless_cols, 160);
        assert_eq!(config.server.headless_rows, 50);
        assert!(!config.server.remember_client_size);

        let invalid: Config = toml::from_str(
            r#"[server]
headless_cols = 0
headless_rows = 50
"#,
        )
        .unwrap();
        assert!(invalid.invalid_headless_size_diagnostic().is_some());
        assert_eq!(
            invalid.headless_size(),
            (
                crate::config::DEFAULT_HEADLESS_COLS,
                crate::config::DEFAULT_HEADLESS_ROWS
            )
        );
    }

    #[test]
    fn advanced_defaults_include_scrollback_limit_bytes() {
        let config = Config::default();
        assert_eq!(
            config.advanced.scrollback_limit_bytes,
            DEFAULT_SCROLLBACK_LIMIT_BYTES
        );
    }

    #[test]
    fn pane_history_persistence_is_opt_in() {
        assert!(!Config::default().experimental.pane_history);

        let toml = r#"
[experimental]
pane_history = true
"#;
        let config: Config = toml::from_str(toml).unwrap();

        assert!(config.experimental.pane_history);
    }

    #[test]
    fn kitty_graphics_default_on_with_stable_opt_out() {
        assert!(Config::default().kitty_graphics_enabled());

        let config: Config = toml::from_str(
            r#"
[terminal]
kitty_graphics = false
"#,
        )
        .unwrap();
        assert!(!config.kitty_graphics_enabled());
    }

    #[test]
    fn legacy_experimental_kitty_graphics_setting_remains_compatible() {
        let disabled: Config = toml::from_str(
            r#"
[experimental]
kitty_graphics = false
"#,
        )
        .unwrap();
        assert!(!disabled.kitty_graphics_enabled());

        let stable_setting_wins: Config = toml::from_str(
            r#"
[terminal]
kitty_graphics = false

[experimental]
kitty_graphics = true
"#,
        )
        .unwrap();
        assert!(!stable_setting_wins.kitty_graphics_enabled());
    }

    #[test]
    fn experimental_config_parses() {
        let toml = r#"
[experimental]
allow_nested = true
kitty_graphics = true
pane_history = true
switch_ascii_input_source_in_prefix = true
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.experimental.allow_nested);
        assert_eq!(config.experimental.kitty_graphics, Some(true));
        assert!(config.kitty_graphics_enabled());
        assert!(config.experimental.pane_history);
        assert!(config.experimental.switch_ascii_input_source_in_prefix);
    }

    #[test]
    fn advanced_config_parses() {
        let toml = r#"
[advanced]
scrollback_limit_bytes = 12345
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.advanced.scrollback_limit_bytes, 12345);
    }

    #[test]
    fn advanced_legacy_scrollback_lines_alias_parses() {
        let toml = r#"
[advanced]
scrollback_lines = 12345
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.advanced.scrollback_limit_bytes, 12345);
    }

    #[test]
    fn sidebar_fog_style_parses_and_an_unknown_value_falls_back_to_lift() {
        let config = Config::default();
        assert_eq!(config.sidebar_fog_style(), SidebarFogStyle::Lift);
        assert!(config.sidebar_fog_diagnostics().is_empty());
        for (value, style) in [
            ("lift", SidebarFogStyle::Lift),
            ("dim", SidebarFogStyle::Dim),
            ("both", SidebarFogStyle::Both),
        ] {
            let config: Config =
                toml::from_str(&format!("[ui]\nsidebar_fog_style = \"{value}\"")).unwrap();
            assert_eq!(config.sidebar_fog_style(), style);
            assert!(config.sidebar_fog_diagnostics().is_empty());
        }
        let config: Config = toml::from_str("[ui]\nsidebar_fog_style = \"blur\"").unwrap();
        assert_eq!(config.sidebar_fog_style(), SidebarFogStyle::Lift);
        assert_eq!(
            config.sidebar_fog_diagnostics(),
            vec!["ui.sidebar_fog_style (\"blur\") is not lift, dim or both; using lift"]
        );
        assert!(toml::from_str::<Config>("[ui]\nsidebar_fog_style = 3").is_err());
    }

    #[test]
    fn inactive_pane_dim_parses_clamps_and_reports() {
        let config = Config::default();
        assert_eq!(config.ui.inactive_pane_dim, 0);
        assert_eq!(config.inactive_pane_dim(), 0);
        assert!(config.collect_diagnostics().is_empty());

        let config: Config = toml::from_str("[ui]\ninactive_pane_dim = 20").unwrap();
        assert_eq!(config.inactive_pane_dim(), 20);
        assert_eq!(config.inactive_pane_dim_diagnostics(), None);

        let config: Config = toml::from_str("[ui]\ninactive_pane_dim = 250").unwrap();
        assert_eq!(config.inactive_pane_dim(), 90);
        assert_eq!(
            config.inactive_pane_dim_diagnostics().as_deref(),
            Some("ui.inactive_pane_dim (250) is outside 0..=90; using 90")
        );
        assert!(config
            .collect_diagnostics()
            .iter()
            .any(|line| line.contains("ui.inactive_pane_dim (250)")));
        let config: Config = toml::from_str("[ui]\ninactive_pane_dim = -3").unwrap();
        assert_eq!(config.inactive_pane_dim(), 0);
        assert!(toml::from_str::<Config>("[ui]\ninactive_pane_dim = \"light\"").is_err());
        // dim_inactive_panes keeps working next to it.
        let config: Config =
            toml::from_str("[ui]\ndim_inactive_panes = true\ninactive_pane_dim = 20").unwrap();
        assert!(config.ui.dim_inactive_panes);
    }

    #[test]
    fn sidebar_fade_parses_clamps_and_reports() {
        let config = Config::default();
        assert_eq!(config.ui.sidebar_fade, vec![85, 55]);
        assert_eq!(config.sidebar_fade(), [85, 55]);
        assert!(config.sidebar_fog_diagnostics().is_empty());

        let config: Config = toml::from_str("[ui]\nsidebar_fade = [70]").unwrap();
        assert_eq!(config.sidebar_fade(), [70, 0]);
        let config: Config = toml::from_str("[ui]\nsidebar_fade = []").unwrap();
        assert_eq!(config.sidebar_fade(), [0, 0]);

        let config: Config = toml::from_str("[ui]\nsidebar_fade = [99, -4, 5]").unwrap();
        assert_eq!(config.sidebar_fade(), [95, 0]);
        assert_eq!(
            config.sidebar_fog_diagnostics(),
            vec![
                "ui.sidebar_fade has 3 entries; only the first 2 are used",
                "ui.sidebar_fade[0] (99) is outside 0..=95; using 95",
                "ui.sidebar_fade[1] (-4) is outside 0..=95; using 0",
            ]
        );
        assert!(toml::from_str::<Config>("[ui]\nsidebar_fade = \"strong\"").is_err());
    }

    #[test]
    fn sidebar_fog_parses_clamps_and_reports() {
        let config = Config::default();
        assert_eq!(config.ui.sidebar_fog, vec![17, 7]);
        assert_eq!(config.sidebar_fog(), ([17, 7], 70));
        assert!(config.sidebar_fog_diagnostics().is_empty());

        let config: Config =
            toml::from_str("[ui]\nsidebar_fog = [25, 3]\nsidebar_fog_tint = 40").unwrap();
        assert_eq!(config.sidebar_fog(), ([25, 3], 40));

        // 0 entries, 1 entry: the missing rows are 0, no fog.
        let config: Config = toml::from_str("[ui]\nsidebar_fog = []").unwrap();
        assert_eq!(config.sidebar_fog().0, [0, 0]);
        let config: Config = toml::from_str("[ui]\nsidebar_fog = [9]").unwrap();
        assert_eq!(config.sidebar_fog().0, [9, 0]);

        // Out of range is clamped and reported; extras are ignored and reported.
        let config: Config =
            toml::from_str("[ui]\nsidebar_fog = [99, -4, 5]\nsidebar_fog_tint = 400").unwrap();
        assert_eq!(config.sidebar_fog(), ([60, 0], 100));
        assert_eq!(
            config.sidebar_fog_diagnostics(),
            vec![
                "ui.sidebar_fog has 3 entries; only the first 2 are used",
                "ui.sidebar_fog[0] (99) is outside 0..=60; using 60",
                "ui.sidebar_fog[1] (-4) is outside 0..=60; using 0",
                "ui.sidebar_fog_tint (400) is outside 0..=100; using 100",
            ]
        );

        // A wrong type is still an error.
        assert!(toml::from_str::<Config>("[ui]\nsidebar_fog = \"strong\"").is_err());
    }

    #[test]
    fn sidebar_overflow_defaults_to_both_and_parses_every_mode() {
        assert_eq!(
            Config::default().ui.sidebar_overflow,
            SidebarOverflowConfig::Both
        );
        for (value, expected) in [
            ("both", SidebarOverflowConfig::Both),
            ("rows", SidebarOverflowConfig::Rows),
            ("fog", SidebarOverflowConfig::Fog),
            ("off", SidebarOverflowConfig::Off),
        ] {
            let toml = format!("[ui]\nsidebar_overflow = \"{value}\"");
            let config: Config = toml::from_str(&toml).unwrap();
            assert_eq!(config.ui.sidebar_overflow, expected, "{value}");
        }
        assert!(toml::from_str::<Config>("[ui]\nsidebar_overflow = \"edges\"").is_err());
        assert!(SidebarOverflowConfig::Both.edge_rows() && SidebarOverflowConfig::Both.fog());
        assert!(SidebarOverflowConfig::Rows.edge_rows() && !SidebarOverflowConfig::Rows.fog());
        assert!(!SidebarOverflowConfig::Fog.edge_rows() && SidebarOverflowConfig::Fog.fog());
        assert!(!SidebarOverflowConfig::Off.edge_rows() && !SidebarOverflowConfig::Off.fog());
    }
}
