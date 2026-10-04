use crate::config::{
    Keybinds, NewTerminalCwdConfig, SoundConfig, TabBarPositionConfig, ToastConfig, ToastDelivery,
};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Direction, Rect};
use ratatui::style::Color;

use crate::detect::AgentState;
use crate::layout::{PaneId, PaneInfo, SplitBorder};
use crate::selection::Selection;

pub(crate) type InstalledPluginRegistry =
    std::collections::HashMap<String, crate::api::schema::InstalledPluginInfo>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PluginPaneRecord {
    pub plugin_id: String,
    pub entrypoint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PopupPaneState {
    pub pane_id: PaneId,
    pub terminal_id: crate::terminal::TerminalId,
    pub width: Option<crate::popup_size::PopupSize>,
    pub height: Option<crate::popup_size::PopupSize>,
}

// ---------------------------------------------------------------------------
// Selection autoscroll types
// ---------------------------------------------------------------------------

/// Direction of automatic scrolling during text selection drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionAutoscrollDirection {
    Up,
    Down,
}

/// State for automatic scrolling during text selection drag.
///
/// When the cursor hovers in the 1-row hot zone at the top or bottom edge
/// of a pane (or outside the pane), this struct captures the direction and
/// last known mouse position so a recurring 30ms tick can continue scrolling
/// and extending the selection even when the mouse is not moving.
#[derive(Clone, Debug)]
pub(crate) struct SelectionAutoscroll {
    pub direction: SelectionAutoscrollDirection,
    pub last_mouse_screen_col: u16,
    pub last_mouse_screen_row: u16,
    pub inner_rect: Rect,
}

#[derive(Clone)]
pub(crate) struct RightClickPassthroughGesture {
    pub pane_info: PaneInfo,
    pub modifiers: KeyModifiers,
}
use crate::terminal_theme::{HostAppearance, TerminalTheme};
use crate::workspace::Workspace;

pub(crate) use super::display_panes::DisplayPanesState;

// ---------------------------------------------------------------------------
// Theme palette — all UI colors in one place, ready for theming
// ---------------------------------------------------------------------------

/// Parsed `[ui.state_colors]` overrides; `None` slots fall back to the theme
/// palette when resolved by [`AppState::state_icon_colors`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StateColorOverrides {
    pub working: Option<Color>,
    pub idle: Option<Color>,
    pub done: Option<Color>,
    pub blocked: Option<Color>,
    pub unknown: Option<Color>,
    pub background: Option<Color>,
}

/// Resolved per-state colors for sidebar state glyphs and state text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateIconColors {
    pub working: Color,
    pub idle: Color,
    pub done: Color,
    pub blocked: Color,
    pub unknown: Color,
    /// Agents parked at their prompt behind work they launched; defaults to
    /// `working`, since that is the state they are still reported in.
    pub background: Color,
}

/// Parsed `[ui.state_symbols]` overrides; `None` slots fall back to the glyph
/// the active indicator style draws, resolved by
/// [`AppState::state_icon_symbols`]. Every stored value is one cell wide.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StateSymbolOverrides {
    pub working: Option<String>,
    pub idle: Option<String>,
    pub done: Option<String>,
    pub blocked: Option<String>,
    pub unknown: Option<String>,
    /// The two frames of the background-work pulse, in order.
    pub background: Option<String>,
    pub background_alt: Option<String>,
}

/// Resolved per-state glyphs for sidebar state icons, each one cell wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateIconSymbols<'a> {
    pub working: &'a str,
    pub idle: &'a str,
    pub done: &'a str,
    pub blocked: &'a str,
    pub unknown: &'a str,
    /// Frames for an agent parked at its prompt while work it launched keeps
    /// running: a slow two-frame pulse, so the row reads as a different kind
    /// of busy than an agent mid-turn.
    pub background: &'a str,
    pub background_alt: &'a str,
}

impl StateIconSymbols<'static> {
    /// The glyph set a `ui.status_indicators` style draws before overrides.
    pub const fn for_style(style: crate::config::StatusIndicatorStyle) -> Self {
        match style {
            crate::config::StatusIndicatorStyle::Dots => Self {
                working: "●",
                idle: "○",
                done: "●",
                blocked: "●",
                unknown: "·",
                background: "■",
                background_alt: "◆",
            },
            // A finished, not-yet-seen agent is an unchecked box the user still
            // has to look at; once seen it is checked off. Keeps the checkmark
            // meaning "handled", as in any todo list and the pre-v0.8.0 icons.
            crate::config::StatusIndicatorStyle::Symbols => Self {
                working: "◐",
                idle: "✓",
                done: "□",
                blocked: "×",
                unknown: "·",
                background: "■",
                background_alt: "◆",
            },
        }
    }
}

impl<'a> StateIconSymbols<'a> {
    /// The background-work frame for a shared spinner frame (a square, then
    /// the square turned 45°): one swap every [`Self::BACKGROUND_PULSE_TICKS`]
    /// frames, so it turns far slower than the working spinner beside it.
    pub const BACKGROUND_PULSE_TICKS: u8 = 8;

    pub fn background_frame(&self, frame: Option<u8>) -> &'a str {
        match frame {
            Some(frame) if (frame / Self::BACKGROUND_PULSE_TICKS) % 2 == 1 => self.background_alt,
            _ => self.background,
        }
    }

    pub fn symbol(&self, state: AgentState, seen: bool) -> &'a str {
        match (state, seen) {
            (AgentState::Blocked, _) => self.blocked,
            (AgentState::Working, _) => self.working,
            (AgentState::Idle, false) => self.done,
            (AgentState::Idle, true) => self.idle,
            (AgentState::Unknown, _) => self.unknown,
        }
    }
}

/// All colors used by the UI. Derived from a base accent color for now,
/// but structured so a full theme system can replace it later.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // all fields defined for theming — some used later
pub struct Palette {
    /// Primary accent (highlight, active borders).
    pub accent: Color,
    /// Background for the tab bar, floating panels, overlays, and modals.
    pub panel_bg: Color,
    /// Optional desktop sidebar background. Reset preserves the terminal background.
    pub sidebar_bg: Color,
    /// Background for the active workspace and focused agent rows.
    pub active_row_bg: Color,
    /// Background for the Navigate-mode cursor row in the sidebar.
    pub selection_bg: Color,
    /// Subtle surface background for selected/focused items.
    pub surface0: Color,
    /// Slightly lighter surface for hover/active states.
    pub surface1: Color,
    /// Very dim surface for separators.
    pub surface_dim: Color,
    /// Muted text (secondary info, numbers).
    pub overlay0: Color,
    /// Slightly brighter overlay text.
    pub overlay1: Color,
    /// Main text color — soft white.
    pub text: Color,
    /// Subdued text (workspace numbers, dim labels).
    pub subtext0: Color,
    /// Branch name / special label color.
    pub mauve: Color,
    /// Done / idle states.
    pub green: Color,
    /// Working / running states.
    pub yellow: Color,
    /// Needs attention / blocked states.
    pub red: Color,
    /// Unseen / done notification accent.
    pub blue: Color,
    /// Notification accent / unseen markers.
    pub teal: Color,
    /// Interrupted / warning states.
    pub peach: Color,
}

/// The yellow of sync mode (fork issue 155): the synced pane borders and the
/// bar chip. A fixed bright yellow, not the theme's, which is the pale one.
pub const SYNC_YELLOW: Color = Color::Rgb(0xff, 0xd6, 0x0a);

impl Palette {
    /// The border of a pane outside the sync group (fork issue 155): a gray
    /// dimmer than the normal inactive border, so "not syncing" reads at a
    /// glance.
    pub fn sync_outsider(&self) -> Color {
        self.surface1
    }

    /// A less saturated red: the theme's red pulled halfway toward its own
    /// grey, a rose on dark themes. The one place the display-panes mode's
    /// inactive borders get their colour, so a custom theme's red carries
    /// through. A red the terminal names rather than gives as RGB cannot be
    /// mixed, so it falls back to the dimmer theme grey.
    pub fn muted_red(&self) -> Color {
        match self.red {
            Color::Rgb(r, g, b) => {
                let grey =
                    (0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b)).round();
                let mix = |channel: u8| ((f32::from(channel) + grey) / 2.0).round() as u8;
                Color::Rgb(mix(r), mix(g), mix(b))
            }
            _ => self.overlay1,
        }
    }

    /// Catppuccin Mocha — the default.
    pub fn catppuccin() -> Self {
        Self {
            accent: Color::Rgb(137, 180, 250), // blue
            panel_bg: Color::Rgb(24, 24, 37),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(30, 30, 46),
            selection_bg: Color::Rgb(49, 50, 68),
            surface0: Color::Rgb(49, 50, 68),
            surface1: Color::Rgb(69, 71, 90),
            surface_dim: Color::Rgb(30, 30, 46),
            overlay0: Color::Rgb(108, 112, 134),
            overlay1: Color::Rgb(127, 132, 156),
            text: Color::Rgb(205, 214, 244),
            subtext0: Color::Rgb(166, 173, 200),
            mauve: Color::Rgb(203, 166, 247),
            green: Color::Rgb(166, 227, 161),
            yellow: Color::Rgb(249, 226, 175),
            red: Color::Rgb(243, 139, 168),
            blue: Color::Rgb(137, 180, 250),
            teal: Color::Rgb(148, 226, 213),
            peach: Color::Rgb(250, 179, 135),
        }
    }

    /// Catppuccin Latte — the light Catppuccin flavor.
    pub fn catppuccin_latte() -> Self {
        Self {
            accent: Color::Rgb(30, 102, 245),
            panel_bg: Color::Rgb(239, 241, 245),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(230, 233, 239),
            selection_bg: Color::Rgb(189, 208, 245),
            surface0: Color::Rgb(204, 208, 218),
            surface1: Color::Rgb(188, 192, 204),
            surface_dim: Color::Rgb(230, 233, 239),
            overlay0: Color::Rgb(156, 160, 176),
            overlay1: Color::Rgb(140, 143, 161),
            text: Color::Rgb(76, 79, 105),
            subtext0: Color::Rgb(108, 111, 133),
            mauve: Color::Rgb(136, 57, 239),
            green: Color::Rgb(64, 160, 43),
            yellow: Color::Rgb(223, 142, 29),
            red: Color::Rgb(210, 15, 57),
            blue: Color::Rgb(30, 102, 245),
            teal: Color::Rgb(23, 146, 153),
            peach: Color::Rgb(254, 100, 11),
        }
    }

    /// Terminal 16-color theme.
    pub fn terminal() -> Self {
        Self {
            accent: Color::Blue,
            panel_bg: Color::Reset,
            sidebar_bg: Color::Reset,
            active_row_bg: Color::DarkGray,
            selection_bg: Color::Reset,
            surface0: Color::Reset,
            surface1: Color::DarkGray,
            surface_dim: Color::DarkGray,
            overlay0: Color::Gray,
            overlay1: Color::White,
            text: Color::Reset,
            subtext0: Color::Gray,
            mauve: Color::Gray,
            green: Color::Green,
            yellow: Color::Yellow,
            red: Color::LightRed,
            blue: Color::Blue,
            teal: Color::Cyan,
            peach: Color::Yellow,
        }
    }

    /// Tokyo Night — blue-purple aesthetic.
    pub fn tokyo_night() -> Self {
        Self {
            accent: Color::Rgb(122, 162, 247), // blue
            panel_bg: Color::Rgb(26, 27, 38),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(35, 38, 54),
            selection_bg: Color::Rgb(45, 54, 80),
            surface0: Color::Rgb(36, 40, 59),
            surface1: Color::Rgb(65, 72, 104),
            surface_dim: Color::Rgb(26, 27, 38),
            overlay0: Color::Rgb(86, 95, 137),
            overlay1: Color::Rgb(105, 113, 150),
            text: Color::Rgb(192, 202, 245),
            subtext0: Color::Rgb(169, 177, 214),
            mauve: Color::Rgb(187, 154, 247),
            green: Color::Rgb(158, 206, 106),
            yellow: Color::Rgb(224, 175, 104),
            red: Color::Rgb(247, 118, 142),
            blue: Color::Rgb(122, 162, 247),
            teal: Color::Rgb(125, 207, 255),
            peach: Color::Rgb(255, 158, 100),
        }
    }

    /// Tokyo Night Day — the light Tokyo Night style.
    pub fn tokyo_night_day() -> Self {
        Self {
            accent: Color::Rgb(46, 125, 233),
            panel_bg: Color::Rgb(225, 226, 231),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(210, 211, 218),
            selection_bg: Color::Rgb(182, 202, 231),
            surface0: Color::Rgb(196, 200, 218),
            surface1: Color::Rgb(168, 174, 203),
            surface_dim: Color::Rgb(210, 211, 218),
            overlay0: Color::Rgb(137, 144, 179),
            overlay1: Color::Rgb(104, 112, 154),
            text: Color::Rgb(55, 96, 191),
            subtext0: Color::Rgb(97, 114, 176),
            mauve: Color::Rgb(120, 71, 189),
            green: Color::Rgb(88, 117, 57),
            yellow: Color::Rgb(140, 108, 62),
            red: Color::Rgb(245, 42, 101),
            blue: Color::Rgb(46, 125, 233),
            teal: Color::Rgb(17, 140, 116),
            peach: Color::Rgb(177, 92, 0),
        }
    }

    /// Dracula — purple/pink/green.
    pub fn dracula() -> Self {
        Self {
            accent: Color::Rgb(189, 147, 249), // purple
            panel_bg: Color::Rgb(40, 42, 54),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(55, 60, 82),
            selection_bg: Color::Rgb(70, 63, 93),
            surface0: Color::Rgb(68, 71, 90),
            surface1: Color::Rgb(98, 114, 164),
            surface_dim: Color::Rgb(40, 42, 54),
            overlay0: Color::Rgb(98, 114, 164),
            overlay1: Color::Rgb(130, 140, 180),
            text: Color::Rgb(248, 248, 242),
            subtext0: Color::Rgb(210, 210, 220),
            mauve: Color::Rgb(255, 121, 198), // pink
            green: Color::Rgb(80, 250, 123),
            yellow: Color::Rgb(241, 250, 140),
            red: Color::Rgb(255, 85, 85),
            blue: Color::Rgb(139, 233, 253), // cyan-ish
            teal: Color::Rgb(139, 233, 253),
            peach: Color::Rgb(255, 184, 108),
        }
    }

    /// Nord — frosty blue palette.
    pub fn nord() -> Self {
        Self {
            accent: Color::Rgb(136, 192, 208), // frost
            panel_bg: Color::Rgb(46, 52, 64),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(67, 76, 94),
            selection_bg: Color::Rgb(64, 80, 93),
            surface0: Color::Rgb(59, 66, 82),
            surface1: Color::Rgb(67, 76, 94),
            surface_dim: Color::Rgb(46, 52, 64),
            overlay0: Color::Rgb(76, 86, 106),
            overlay1: Color::Rgb(100, 110, 130),
            text: Color::Rgb(236, 239, 244),
            subtext0: Color::Rgb(216, 222, 233),
            mauve: Color::Rgb(180, 142, 173),
            green: Color::Rgb(163, 190, 140),
            yellow: Color::Rgb(235, 203, 139),
            red: Color::Rgb(191, 97, 106),
            blue: Color::Rgb(129, 161, 193),
            teal: Color::Rgb(143, 188, 187),
            peach: Color::Rgb(208, 135, 112),
        }
    }

    /// Gruvbox Dark — warm retro palette.
    pub fn gruvbox() -> Self {
        Self {
            accent: Color::Rgb(215, 153, 33), // yellow
            panel_bg: Color::Rgb(40, 40, 40),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(50, 49, 48),
            selection_bg: Color::Rgb(75, 63, 39),
            surface0: Color::Rgb(60, 56, 54),
            surface1: Color::Rgb(80, 73, 69),
            surface_dim: Color::Rgb(40, 40, 40),
            overlay0: Color::Rgb(146, 131, 116),
            overlay1: Color::Rgb(168, 153, 132),
            text: Color::Rgb(235, 219, 178),
            subtext0: Color::Rgb(213, 196, 161),
            mauve: Color::Rgb(211, 134, 155),
            green: Color::Rgb(184, 187, 38),
            yellow: Color::Rgb(250, 189, 47),
            red: Color::Rgb(251, 73, 52),
            blue: Color::Rgb(131, 165, 152),
            teal: Color::Rgb(142, 192, 124),
            peach: Color::Rgb(254, 128, 25),
        }
    }

    /// Gruvbox Light — the light retro palette.
    pub fn gruvbox_light() -> Self {
        Self {
            accent: Color::Rgb(7, 102, 120),
            panel_bg: Color::Rgb(251, 241, 199),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(242, 229, 188),
            selection_bg: Color::Rgb(235, 219, 178),
            surface0: Color::Rgb(235, 219, 178),
            surface1: Color::Rgb(213, 196, 161),
            surface_dim: Color::Rgb(242, 229, 188),
            overlay0: Color::Rgb(146, 131, 116),
            overlay1: Color::Rgb(124, 111, 100),
            text: Color::Rgb(60, 56, 54),
            subtext0: Color::Rgb(80, 73, 69),
            mauve: Color::Rgb(143, 63, 113),
            green: Color::Rgb(121, 116, 14),
            yellow: Color::Rgb(181, 118, 20),
            red: Color::Rgb(157, 0, 6),
            blue: Color::Rgb(7, 102, 120),
            teal: Color::Rgb(66, 123, 88),
            peach: Color::Rgb(175, 58, 3),
        }
    }

    /// One Dark — Atom's classic dark theme.
    pub fn one_dark() -> Self {
        Self {
            accent: Color::Rgb(97, 175, 239), // blue
            panel_bg: Color::Rgb(40, 44, 52),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(49, 54, 64),
            selection_bg: Color::Rgb(51, 70, 89),
            surface0: Color::Rgb(44, 49, 58),
            surface1: Color::Rgb(62, 68, 81),
            surface_dim: Color::Rgb(40, 44, 52),
            overlay0: Color::Rgb(92, 99, 112),
            overlay1: Color::Rgb(115, 122, 135),
            text: Color::Rgb(171, 178, 191),
            subtext0: Color::Rgb(150, 156, 168),
            mauve: Color::Rgb(198, 120, 221),
            green: Color::Rgb(152, 195, 121),
            yellow: Color::Rgb(229, 192, 123),
            red: Color::Rgb(224, 108, 117),
            blue: Color::Rgb(97, 175, 239),
            teal: Color::Rgb(86, 182, 194),
            peach: Color::Rgb(209, 154, 102),
        }
    }

    /// One Light — Atom's classic light theme.
    pub fn one_light() -> Self {
        Self {
            accent: Color::Rgb(64, 120, 242),
            panel_bg: Color::Rgb(250, 250, 250),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(216, 219, 226),
            selection_bg: Color::Rgb(205, 219, 248),
            surface0: Color::Rgb(240, 240, 241),
            surface1: Color::Rgb(229, 229, 230),
            surface_dim: Color::Rgb(245, 245, 246),
            overlay0: Color::Rgb(160, 161, 167),
            overlay1: Color::Rgb(104, 107, 119),
            text: Color::Rgb(56, 58, 66),
            subtext0: Color::Rgb(104, 107, 119),
            mauve: Color::Rgb(166, 38, 164),
            green: Color::Rgb(80, 161, 79),
            yellow: Color::Rgb(193, 132, 1),
            red: Color::Rgb(228, 86, 73),
            blue: Color::Rgb(64, 120, 242),
            teal: Color::Rgb(1, 132, 188),
            peach: Color::Rgb(152, 104, 1),
        }
    }

    /// Solarized Dark — Ethan Schoonover's classic.
    pub fn solarized() -> Self {
        Self {
            accent: Color::Rgb(38, 139, 210), // blue
            panel_bg: Color::Rgb(0, 43, 54),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(22, 75, 87),
            selection_bg: Color::Rgb(8, 62, 85),
            surface0: Color::Rgb(7, 54, 66),
            surface1: Color::Rgb(88, 110, 117),
            surface_dim: Color::Rgb(0, 43, 54),
            overlay0: Color::Rgb(88, 110, 117),
            overlay1: Color::Rgb(101, 123, 131),
            text: Color::Rgb(147, 161, 161),
            subtext0: Color::Rgb(131, 148, 150),
            mauve: Color::Rgb(211, 54, 130),
            green: Color::Rgb(133, 153, 0),
            yellow: Color::Rgb(181, 137, 0),
            red: Color::Rgb(220, 50, 47),
            blue: Color::Rgb(38, 139, 210),
            teal: Color::Rgb(42, 161, 152),
            peach: Color::Rgb(203, 75, 22),
        }
    }

    /// Solarized Light — Ethan Schoonover's light variant.
    pub fn solarized_light() -> Self {
        Self {
            accent: Color::Rgb(38, 139, 210),
            panel_bg: Color::Rgb(253, 246, 227),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(238, 232, 213),
            selection_bg: Color::Rgb(201, 220, 223),
            surface0: Color::Rgb(238, 232, 213),
            surface1: Color::Rgb(147, 161, 161),
            surface_dim: Color::Rgb(238, 232, 213),
            overlay0: Color::Rgb(147, 161, 161),
            overlay1: Color::Rgb(88, 110, 117),
            text: Color::Rgb(101, 123, 131),
            subtext0: Color::Rgb(131, 148, 150),
            mauve: Color::Rgb(211, 54, 130),
            green: Color::Rgb(133, 153, 0),
            yellow: Color::Rgb(181, 137, 0),
            red: Color::Rgb(220, 50, 47),
            blue: Color::Rgb(38, 139, 210),
            teal: Color::Rgb(42, 161, 152),
            peach: Color::Rgb(203, 75, 22),
        }
    }

    /// Kanagawa — inspired by Katsushika Hokusai.
    pub fn kanagawa() -> Self {
        Self {
            accent: Color::Rgb(126, 156, 216), // blue
            panel_bg: Color::Rgb(31, 31, 40),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(54, 54, 70),
            selection_bg: Color::Rgb(50, 56, 75),
            surface0: Color::Rgb(42, 42, 55),
            surface1: Color::Rgb(54, 54, 70),
            surface_dim: Color::Rgb(31, 31, 40),
            overlay0: Color::Rgb(114, 113, 105),
            overlay1: Color::Rgb(135, 134, 125),
            text: Color::Rgb(220, 215, 186),
            subtext0: Color::Rgb(200, 195, 170),
            mauve: Color::Rgb(149, 127, 184),
            green: Color::Rgb(118, 148, 106),
            yellow: Color::Rgb(192, 163, 110),
            red: Color::Rgb(195, 64, 67),
            blue: Color::Rgb(126, 156, 216),
            teal: Color::Rgb(127, 180, 202),
            peach: Color::Rgb(255, 160, 102),
        }
    }

    /// Kanagawa Lotus — the light Kanagawa variant.
    pub fn kanagawa_lotus() -> Self {
        Self {
            accent: Color::Rgb(77, 105, 155),
            panel_bg: Color::Rgb(242, 236, 188),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(213, 206, 163),
            selection_bg: Color::Rgb(220, 213, 172),
            surface0: Color::Rgb(220, 213, 172),
            surface1: Color::Rgb(201, 203, 209),
            surface_dim: Color::Rgb(213, 206, 163),
            overlay0: Color::Rgb(160, 156, 172),
            overlay1: Color::Rgb(138, 137, 128),
            text: Color::Rgb(84, 84, 100),
            subtext0: Color::Rgb(67, 67, 108),
            mauve: Color::Rgb(98, 76, 131),
            green: Color::Rgb(111, 137, 78),
            yellow: Color::Rgb(119, 113, 63),
            red: Color::Rgb(200, 64, 83),
            blue: Color::Rgb(77, 105, 155),
            teal: Color::Rgb(78, 140, 162),
            peach: Color::Rgb(204, 109, 0),
        }
    }

    /// Rosé Pine — muted, elegant.
    pub fn rose_pine() -> Self {
        Self {
            accent: Color::Rgb(196, 167, 231), // iris
            panel_bg: Color::Rgb(25, 23, 36),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(38, 35, 58),
            selection_bg: Color::Rgb(59, 52, 75),
            surface0: Color::Rgb(31, 29, 46),
            surface1: Color::Rgb(38, 35, 58),
            surface_dim: Color::Rgb(38, 35, 58),
            overlay0: Color::Rgb(110, 106, 134),
            overlay1: Color::Rgb(144, 140, 170),
            text: Color::Rgb(224, 222, 244),
            subtext0: Color::Rgb(200, 197, 220),
            mauve: Color::Rgb(196, 167, 231),  // iris
            green: Color::Rgb(49, 116, 143),   // pine
            yellow: Color::Rgb(246, 193, 119), // gold
            red: Color::Rgb(235, 111, 146),    // love
            blue: Color::Rgb(49, 116, 143),    // pine
            teal: Color::Rgb(156, 207, 216),   // foam
            peach: Color::Rgb(234, 154, 151),  // rose
        }
    }

    /// Rosé Pine Dawn — the light Rosé Pine variant.
    pub fn rose_pine_dawn() -> Self {
        Self {
            accent: Color::Rgb(144, 122, 169),
            panel_bg: Color::Rgb(250, 244, 237),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(227, 217, 207),
            selection_bg: Color::Rgb(242, 233, 225),
            surface0: Color::Rgb(242, 233, 225),
            surface1: Color::Rgb(255, 250, 243),
            surface_dim: Color::Rgb(242, 233, 225),
            overlay0: Color::Rgb(152, 147, 165),
            overlay1: Color::Rgb(121, 117, 147),
            text: Color::Rgb(70, 66, 97),
            subtext0: Color::Rgb(121, 117, 147),
            mauve: Color::Rgb(144, 122, 169),
            green: Color::Rgb(40, 105, 131),
            yellow: Color::Rgb(234, 157, 52),
            red: Color::Rgb(180, 99, 122),
            blue: Color::Rgb(40, 105, 131),
            teal: Color::Rgb(86, 148, 159),
            peach: Color::Rgb(215, 130, 126),
        }
    }

    /// Vesper — minimal high-contrast monochrome with peach and mint accents.
    pub fn vesper() -> Self {
        Self {
            accent: Color::Rgb(255, 199, 153),
            panel_bg: Color::Rgb(26, 26, 26),
            sidebar_bg: Color::Reset,
            active_row_bg: Color::Rgb(16, 16, 16),
            selection_bg: Color::Rgb(35, 35, 35),
            surface0: Color::Rgb(35, 35, 35),
            surface1: Color::Rgb(40, 40, 40),
            surface_dim: Color::Rgb(16, 16, 16),
            overlay0: Color::Rgb(92, 92, 92),
            overlay1: Color::Rgb(126, 126, 126),
            text: Color::Rgb(255, 255, 255),
            subtext0: Color::Rgb(160, 160, 160),
            mauve: Color::Rgb(255, 209, 168),
            green: Color::Rgb(153, 255, 228),
            yellow: Color::Rgb(255, 199, 153),
            red: Color::Rgb(255, 128, 128),
            blue: Color::Rgb(176, 176, 176),
            teal: Color::Rgb(102, 221, 204),
            peach: Color::Rgb(255, 199, 153),
        }
    }

    /// Resolve a theme by name. Returns None for unknown names.
    pub fn from_name(name: &str) -> Option<Self> {
        match crate::config::canonical_theme_name(name)? {
            "catppuccin" => Some(Self::catppuccin()),
            "catppuccin-latte" => Some(Self::catppuccin_latte()),
            "terminal" => Some(Self::terminal()),
            "tokyo-night" => Some(Self::tokyo_night()),
            "tokyo-night-day" => Some(Self::tokyo_night_day()),
            "dracula" => Some(Self::dracula()),
            "nord" => Some(Self::nord()),
            "gruvbox" => Some(Self::gruvbox()),
            "gruvbox-light" => Some(Self::gruvbox_light()),
            "one-dark" => Some(Self::one_dark()),
            "one-light" => Some(Self::one_light()),
            "solarized" => Some(Self::solarized()),
            "solarized-light" => Some(Self::solarized_light()),
            "kanagawa" => Some(Self::kanagawa()),
            "kanagawa-lotus" => Some(Self::kanagawa_lotus()),
            "rose-pine" => Some(Self::rose_pine()),
            "rose-pine-dawn" => Some(Self::rose_pine_dawn()),
            "vesper" => Some(Self::vesper()),
            _ => None,
        }
    }

    /// Apply custom color overrides on top of this palette.
    pub fn with_overrides(mut self, custom: &crate::config::CustomThemeColors) -> Self {
        use crate::config::parse_color;
        if let Some(c) = &custom.accent {
            self.accent = parse_color(c);
        }
        if let Some(c) = &custom.panel_bg {
            self.panel_bg = parse_color(c);
        }
        if let Some(c) = &custom.sidebar_bg {
            self.sidebar_bg = parse_color(c);
        }
        if let Some(c) = &custom.active_row_bg {
            self.active_row_bg = parse_color(c);
        }
        if let Some(c) = &custom.selection_bg {
            self.selection_bg = parse_color(c);
        }
        if let Some(c) = &custom.surface0 {
            self.surface0 = parse_color(c);
        }
        if let Some(c) = &custom.surface1 {
            self.surface1 = parse_color(c);
        }
        if let Some(c) = &custom.surface_dim {
            self.surface_dim = parse_color(c);
        }
        if let Some(c) = &custom.overlay0 {
            self.overlay0 = parse_color(c);
        }
        if let Some(c) = &custom.overlay1 {
            self.overlay1 = parse_color(c);
        }
        if let Some(c) = &custom.text {
            self.text = parse_color(c);
        }
        if let Some(c) = &custom.subtext0 {
            self.subtext0 = parse_color(c);
        }
        if let Some(c) = &custom.mauve {
            self.mauve = parse_color(c);
        }
        if let Some(c) = &custom.green {
            self.green = parse_color(c);
        }
        if let Some(c) = &custom.yellow {
            self.yellow = parse_color(c);
        }
        if let Some(c) = &custom.red {
            self.red = parse_color(c);
        }
        if let Some(c) = &custom.blue {
            self.blue = parse_color(c);
        }
        if let Some(c) = &custom.teal {
            self.teal = parse_color(c);
        }
        if let Some(c) = &custom.peach {
            self.peach = parse_color(c);
        }
        self
    }

    pub fn with_mode_overrides(mut self, custom: &crate::config::ModeThemeColors) -> Self {
        use crate::config::parse_color;
        if let Some(c) = &custom.accent {
            self.accent = parse_color(c);
        }
        if let Some(c) = &custom.panel_bg {
            self.panel_bg = parse_color(c);
        }
        if let Some(c) = &custom.sidebar_bg {
            self.sidebar_bg = parse_color(c);
        }
        if let Some(c) = &custom.active_row_bg {
            self.active_row_bg = parse_color(c);
        }
        if let Some(c) = &custom.selection_bg {
            self.selection_bg = parse_color(c);
        }
        if let Some(c) = &custom.surface0 {
            self.surface0 = parse_color(c);
        }
        if let Some(c) = &custom.surface1 {
            self.surface1 = parse_color(c);
        }
        if let Some(c) = &custom.surface_dim {
            self.surface_dim = parse_color(c);
        }
        if let Some(c) = &custom.overlay0 {
            self.overlay0 = parse_color(c);
        }
        if let Some(c) = &custom.overlay1 {
            self.overlay1 = parse_color(c);
        }
        if let Some(c) = &custom.text {
            self.text = parse_color(c);
        }
        if let Some(c) = &custom.subtext0 {
            self.subtext0 = parse_color(c);
        }
        if let Some(c) = &custom.mauve {
            self.mauve = parse_color(c);
        }
        if let Some(c) = &custom.green {
            self.green = parse_color(c);
        }
        if let Some(c) = &custom.yellow {
            self.yellow = parse_color(c);
        }
        if let Some(c) = &custom.red {
            self.red = parse_color(c);
        }
        if let Some(c) = &custom.blue {
            self.blue = parse_color(c);
        }
        if let Some(c) = &custom.teal {
            self.teal = parse_color(c);
        }
        if let Some(c) = &custom.peach {
            self.peach = parse_color(c);
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceCardArea {
    pub ws_idx: usize,
    pub rect: Rect,
    pub indented: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeCreateState {
    pub source_workspace_id: String,
    pub source_checkout_path: std::path::PathBuf,
    pub source_existing_membership: Option<crate::workspace::WorktreeSpaceMembership>,
    pub source_repo_root: std::path::PathBuf,
    pub repo_key: String,
    pub repo_name: String,
    pub branch: String,
    pub checkout_path: std::path::PathBuf,
    pub error: Option<String>,
    pub creating: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRemoveState {
    pub workspace_id: String,
    pub repo_root: std::path::PathBuf,
    pub path: std::path::PathBuf,
    pub error: Option<String>,
    pub removing: bool,
    pub force_confirmation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeOpenEntry {
    pub path: std::path::PathBuf,
    pub branch: Option<String>,
    pub is_linked_worktree: bool,
    pub already_open_ws_idx: Option<usize>,
}

impl WorktreeOpenEntry {
    pub(crate) fn display_name(&self) -> String {
        self.branch.clone().unwrap_or_else(|| {
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .unwrap_or_else(|| self.path.display().to_string())
        })
    }

    pub(crate) fn status_label(&self) -> &'static str {
        if self.already_open_ws_idx.is_some() {
            "open"
        } else if self.branch.is_some() {
            ""
        } else if self.is_linked_worktree {
            "detached"
        } else {
            "root"
        }
    }

    fn search_text(&self) -> String {
        format!(
            "{} {} {} {}",
            self.display_name(),
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default(),
            self.path.display(),
            self.status_label()
        )
        .to_lowercase()
    }

    fn matches_query(&self, query: &str) -> bool {
        text_matches_query(query, &self.search_text())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeOpenState {
    pub source_workspace_id: String,
    pub source_existing_membership: Option<crate::workspace::WorktreeSpaceMembership>,
    pub source_checkout_path: std::path::PathBuf,
    pub source_repo_root: std::path::PathBuf,
    pub repo_key: String,
    pub repo_name: String,
    pub entries: Vec<WorktreeOpenEntry>,
    pub selected: usize,
    pub query: String,
    pub search_focused: bool,
    pub error: Option<String>,
}

impl WorktreeOpenState {
    pub(crate) fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.trim();
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| {
                (query.is_empty() || entry.matches_query(query)).then_some(idx)
            })
            .collect()
    }

    pub(crate) fn selected_entry_index(&self) -> Option<usize> {
        let indices = self.filtered_indices();
        if indices.contains(&self.selected) {
            Some(self.selected)
        } else {
            indices.first().copied()
        }
    }

    pub(crate) fn normalize_selection(&mut self) {
        if let Some(selected) = self.selected_entry_index() {
            self.selected = selected;
        }
    }

    pub(crate) fn select_previous_filtered(&mut self) {
        let indices = self.filtered_indices();
        let Some(current) = self.selected_entry_index() else {
            return;
        };
        let pos = indices.iter().position(|idx| *idx == current).unwrap_or(0);
        self.selected = indices[pos.saturating_sub(1)];
    }

    pub(crate) fn select_next_filtered(&mut self) {
        let indices = self.filtered_indices();
        let Some(current) = self.selected_entry_index() else {
            return;
        };
        let pos = indices.iter().position(|idx| *idx == current).unwrap_or(0);
        self.selected = indices[(pos + 1).min(indices.len().saturating_sub(1))];
    }
}

pub(crate) use crate::ui::overlay::text_matches_query;

/// Computed view geometry — derived from AppState + terminal size.
/// Updated before each render, consumed by render and mouse handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewLayout {
    Desktop,
    Mobile,
}

pub struct ViewState {
    pub layout: ViewLayout,
    pub sidebar_rect: Rect,
    pub workspace_card_areas: Vec<WorkspaceCardArea>,
    pub tab_bar_rect: Rect,
    pub tab_hit_areas: Vec<Rect>,
    pub tab_scroll_left_hit_area: Rect,
    pub tab_scroll_right_hit_area: Rect,
    pub new_tab_hit_area: Rect,
    pub todo_hit_area: Rect,
    pub notification_hit_area: Rect,
    pub terminal_area: Rect,
    pub mobile_header_rect: Rect,
    pub mobile_menu_hit_area: Rect,
    pub toast_hit_area: Rect,
    pub pane_infos: Vec<PaneInfo>,
    pub split_borders: Vec<SplitBorder>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Onboarding,
    ReleaseNotes,
    ProductAnnouncement,
    Navigate,
    Prefix,
    Copy,
    AppScroll,
    Terminal,
    RenameWorkspace,
    RenameTab,
    RenamePane,
    NewLinkedWorktree,
    OpenExistingWorktree,
    PaneMoveTargetPicker,
    ConfirmRemoveWorktree,
    Resize,
    ConfirmClose,
    ContextMenu,
    Settings,
    GlobalMenu,
    KeybindHelp,
    Navigator,
    NotificationCenter,
    PaneTodos,
    PaneTodoEdit,
    TodoBoard,
    DisplayPanes,
}

impl Mode {
    pub(crate) fn mouse_motion_changes_view(self) -> bool {
        matches!(self, Self::GlobalMenu | Self::ContextMenu | Self::Navigator)
    }

    /// Whether keys in this mode are commands/navigation (an ASCII input source is wanted) rather
    /// than free text. Used by `sync_prefix_input_source` (gated by
    /// `switch_ascii_input_source_in_prefix`) so multi-level prefix commands keep ASCII until they
    /// return to the terminal.
    ///
    /// Only the non-overlay modes are listed here. Every overlay's answer is declared beside its
    /// variant in the `overlays!` list and derived through [`OverlayKind::mode`], so a new overlay
    /// cannot silently fall off an allowlist it never knew about.
    ///
    /// Known limitation: the search boxes in `Navigator` and `KeybindHelp` are also held on ASCII,
    /// since this `Mode`-level predicate can't see `search_focused` (non-ASCII filtering there
    /// would need a runtime check).
    pub(crate) fn wants_ascii_input(self) -> bool {
        matches!(
            self,
            Mode::Prefix | Mode::Navigate | Mode::Copy | Mode::Resize | Mode::ConfirmClose
        ) || OverlayKind::ALL
            .iter()
            .any(|kind| kind.mode() == self && kind.wants_ascii_input())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NavigatorTarget {
    Workspace {
        ws_idx: usize,
    },
    Tab {
        ws_idx: usize,
        tab_idx: usize,
    },
    Pane {
        ws_idx: usize,
        tab_idx: usize,
        pane_id: PaneId,
    },
    /// Synthetic row offered only while choosing a todo's link target:
    /// picking it clears the link instead of pointing it at a pane.
    ClearLink,
}

/// What an open navigator is being used for. `Goto` focuses whatever is
/// picked; a selection purpose hands the choice back to what opened it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum NavigatorPurpose {
    #[default]
    Goto,
    /// Choosing a link target for the todo open in the edit modal.
    PaneTodoLink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavigatorRow {
    pub target: NavigatorTarget,
    pub depth: u8,
    pub label: String,
    pub meta: String,
    pub status: AgentState,
    pub seen: bool,
    pub is_current: bool,
    pub is_workspace: bool,
    pub is_tab: bool,
    pub expanded: bool,
    /// The pane's public identifier, for pane rows only. It is what a link
    /// picker row would stage, so the picker leads with it.
    pub public_pane_id: Option<String>,
    pub search_text: String,
    /// Whether this row itself matched the active query/state filter, as
    /// opposed to being included as ancestor context or cascaded subtree of a
    /// matching workspace or tab. Always true when no filter is active.
    pub matched: bool,
}

/// One rendered line in the navigator body. Spacer lines separate workspace
/// groups visually and are not selectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavigatorDisplayLine {
    Spacer,
    Row(usize),
}

pub(crate) fn navigator_display_lines(rows: &[NavigatorRow]) -> Vec<NavigatorDisplayLine> {
    let mut lines = Vec::with_capacity(rows.len().saturating_mul(2));
    for (idx, row) in rows.iter().enumerate() {
        if row.is_workspace && !lines.is_empty() {
            lines.push(NavigatorDisplayLine::Spacer);
        }
        lines.push(NavigatorDisplayLine::Row(idx));
    }
    lines
}

pub(crate) fn navigator_display_index_of_row(
    lines: &[NavigatorDisplayLine],
    row_idx: usize,
) -> Option<usize> {
    lines
        .iter()
        .position(|line| *line == NavigatorDisplayLine::Row(row_idx))
}

pub(crate) fn navigator_first_row_at_or_after(
    lines: &[NavigatorDisplayLine],
    line_idx: usize,
) -> Option<usize> {
    lines.get(line_idx..)?.iter().find_map(|line| match line {
        NavigatorDisplayLine::Row(idx) => Some(*idx),
        NavigatorDisplayLine::Spacer => None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavigatorStateFilter {
    Blocked,
    Working,
    Idle,
    Done,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NavigatorState {
    /// The kit's list search: the query and whether it has the keyboard.
    pub search: crate::ui::overlay::ListSearch,
    pub selected: usize,
    pub scroll: usize,
    /// The width its rows asked for when it opened, border included; see
    /// `crate::ui::navigator_columns`.
    pub content_width: u16,
    /// The status column its rows measured when it opened, padding included;
    /// the renderer draws this width, not a guess from the box width.
    pub status_width: u16,
    pub state_filter: Option<NavigatorStateFilter>,
    pub expanded_workspaces: std::collections::HashSet<String>,
    /// Consulted at activation only, never threaded through rendering of the
    /// rows themselves.
    pub purpose: NavigatorPurpose,
    /// The todo edit this picker was opened from, suspended here for the
    /// duration and handed back when the picker closes.
    ///
    /// It used to live in a parallel field that outlived its own mode, which
    /// is what made "two overlays open at once" representable. Carrying it on
    /// the overlay that suspended it makes the return path explicit instead.
    pub suspended_pane_todo_edit: Option<PaneTodoEditState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CopyModeState {
    pub pane_id: PaneId,
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub entry_offset_from_bottom: usize,
    pub selection: Option<CopyModeSelection>,
    pub search: CopyModeSearchState,
}

/// Alt-screen scroll passthrough: scroll keys are translated and forwarded to
/// the pinned pane's application instead of moving Herdr's own viewport, which
/// an alternate-screen application keeps empty of scrollback by definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppScrollState {
    pub pane_id: PaneId,
}

/// One send the passthrough mode queued toward its application: a whole key,
/// or a line-granular wheel tick. Line scrolling cannot forward a key — arrow
/// keys mean prompt history in shell-like TUIs — so it synthesizes a wheel
/// event instead, encoded per the pane's own mouse protocol at drain time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppScrollSend {
    Key(crate::input::TerminalKey),
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyModeSelection {
    Character,
    Linewise { anchor_row: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyModeSearchDirection {
    Forward,
    Backward,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CopyModeSearchPrompt {
    pub direction: CopyModeSearchDirection,
    pub query: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CopyModeSearchState {
    pub prompt: Option<CopyModeSearchPrompt>,
    pub query: String,
    pub direction: Option<CopyModeSearchDirection>,
    pub matches: Vec<crate::pane::TerminalTextMatch>,
    pub current: Option<usize>,
    pub geometry: Option<(u16, u16)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentPanelSort {
    #[default]
    Spaces,
    Priority,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspaceSort {
    #[default]
    Manual,
    Priority,
}

// ---------------------------------------------------------------------------
// Settings UI state
// ---------------------------------------------------------------------------

/// Which section of the settings panel is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Theme,
    Indicators,
    Sound,
    Toast,
    PaneLabels,
    Integrations,
}

impl SettingsSection {
    pub const ALL: &[Self] = &[
        Self::Theme,
        Self::Indicators,
        Self::Sound,
        Self::Toast,
        Self::PaneLabels,
        Self::Integrations,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::Indicators => "indicators",
            Self::Sound => "sound",
            Self::Toast => "toasts",
            Self::PaneLabels => "pane labels",
            Self::Integrations => "integrations",
        }
    }
}

/// All built-in theme names in display order.
pub const THEME_NAMES: &[&str] = crate::config::THEME_NAMES;

/// Characters an overlay search box accepts. A query is a filter, not a
/// document; the cap only exists because a cursor-bearing field takes one.
pub(crate) const SEARCH_QUERY_MAX_CHARS: usize = 256;

/// Characters a name field accepts. Names — a workspace, a tab, a pane, a
/// branch — are short; the cap only exists because a cursor-bearing field
/// takes one, and it is far past anything a name should be.
pub(crate) const NAME_INPUT_MAX_CHARS: usize = 512;

/// The smallest client size, (cols, rows), remembered as the no-client size.
/// A quick look from a phone or a tiny split is used while it is attached,
/// but must not leave every pane that small once it detaches.
pub(crate) const REMEMBERED_CLIENT_SIZE_FLOOR: (u16, u16) = (80, 24);

/// `size` if it is big enough to remember, or `None`. A zero or corrupt size
/// from a session file falls below the floor too.
pub(crate) fn rememberable_client_size(size: (u16, u16)) -> Option<(u16, u16)> {
    let (min_cols, min_rows) = REMEMBERED_CLIENT_SIZE_FLOOR;
    (size.0 >= min_cols && size.1 >= min_rows).then_some(size)
}

/// The overlay kit's list cursor, re-exported so overlay state can name it
/// without every module reaching into `crate::ui`.
pub(crate) use crate::ui::overlay::ListCursor;

/// Where the pane-move picker can send a pane. Maps 1:1 onto
/// [`crate::api::schema::PaneMoveDestination`] at dispatch, so the picker only
/// has to pick — the API's vocabulary is already correct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneMoveTarget {
    /// An existing tab, in the pane's own space or another one.
    Tab { tab_id: String },
    /// A new tab in a named space.
    NewTab { workspace_id: String },
    /// A new space, created by the move.
    NewSpace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneMoveTargetEntry {
    /// Space this destination belongs to, `None` for the new-space entry.
    pub workspace_id: Option<String>,
    /// Display number of a tab destination within its space; unused otherwise.
    pub number: usize,
    /// Tab label, when the destination is an existing named tab.
    pub label: String,
    pub target: PaneMoveTarget,
    /// What the row shows about the place, beyond its name.
    pub facts: PaneMoveTabFacts,
}

/// What a tab row shows about the tab: its space, its panes and its state.
/// Read once when the picker opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneMoveTabFacts {
    /// The space's name as its heading shows it; empty for the new-space entry.
    pub space: String,
    /// The tab's panes, by the label the sidebar gives them, in layout order.
    /// Empty for the new-tab and new-space entries.
    pub pane_names: Vec<String>,
    pub status: crate::detect::AgentState,
    pub seen: bool,
}

impl Default for PaneMoveTabFacts {
    fn default() -> Self {
        Self {
            space: String::new(),
            pane_names: Vec::new(),
            status: crate::detect::AgentState::Unknown,
            seen: true,
        }
    }
}

/// One row of the picker. Only [`PaneMoveTargetItem::Destination`] can be
/// selected; the rest are drawn for orientation, so render and selection read
/// the same list instead of deriving headings twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneMoveTargetItem {
    SpaceHeading {
        label: String,
        pane_count: usize,
        /// The space's activity (`1 working`), empty when there is none.
        activity: String,
        status: crate::detect::AgentState,
        seen: bool,
    },
    /// The tab the pane is in now. Moving there does nothing, so it is shown
    /// greyed, in its place, and never selected or counted.
    Here(PaneMoveTargetEntry),
    Destination(PaneMoveTargetEntry),
    /// The blank line between two spaces.
    Gap,
}

impl PaneMoveTargetItem {
    /// A heading with no facts beyond its name, for tests that only care
    /// about the list's shape.
    #[cfg(test)]
    pub fn heading(label: &str) -> Self {
        Self::SpaceHeading {
            label: label.to_string(),
            pane_count: 0,
            activity: String::new(),
            status: crate::detect::AgentState::Unknown,
            seen: true,
        }
    }

    /// Whether this row belongs under a space heading as one of its branches.
    pub fn is_branch(&self) -> bool {
        match self {
            Self::Here(_) => true,
            Self::Destination(entry) => entry.workspace_id.is_some(),
            Self::SpaceHeading { .. } | Self::Gap => false,
        }
    }
}

/// The move picker's footer buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneMoveTargetPickerButton {
    Move,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneMoveTargetPickerState {
    pub source_pane_id: String,
    /// The moving pane's label and where it is (`herdr › tab 1 · cc`), for
    /// the title. Empty in tests that build a picker by hand.
    pub source_label: String,
    pub source_place: String,
    /// Every destination the picker offers, grouped under its space's heading.
    /// The list is `items`, this filtered by the search; the picker is sized
    /// from this, so typing a query does not resize it.
    pub all_items: Vec<PaneMoveTargetItem>,
    pub items: Vec<PaneMoveTargetItem>,
    pub list: ListCursor,
    pub search: crate::ui::overlay::ListSearch,
}

impl PaneMoveTargetPickerState {
    /// Builds a picker selecting the first destination, stepping over any
    /// leading space heading.
    pub fn new(source_pane_id: String, items: Vec<PaneMoveTargetItem>) -> Self {
        let mut picker = Self {
            source_pane_id,
            source_label: String::new(),
            source_place: String::new(),
            all_items: items.clone(),
            items,
            list: ListCursor::default(),
            search: crate::ui::overlay::ListSearch::default(),
        };
        picker.select_first_destination();
        picker
    }

    fn select_first_destination(&mut self) {
        let selected = self
            .items
            .iter()
            .position(|item| matches!(item, PaneMoveTargetItem::Destination(_)))
            .unwrap_or(0);
        self.list = ListCursor::new(selected);
    }

    /// Rebuild the list from the search and select the first match.
    ///
    /// A destination matches on its row text and its panes' names with its
    /// space's name in front, so `ctx logs` finds CONTEXT's `logs` tab and
    /// `keyboard` finds the tab holding `keyboard-shortcuts`. A space heading
    /// stays while any of its destinations match, and a heading that matches
    /// keeps all of them: a space is a place, and finding it by name offers
    /// everything in it. The pane's own tab stays with its space. Surviving
    /// spaces are kept apart by a gap, as in the full list.
    pub fn refilter(&mut self) {
        if !self.search.is_active() {
            self.items = self.all_items.clone();
            self.select_first_destination();
            return;
        }
        let mut groups: Vec<Vec<PaneMoveTargetItem>> = Vec::new();
        let mut idx = 0;
        while idx < self.all_items.len() {
            match &self.all_items[idx] {
                PaneMoveTargetItem::SpaceHeading { label, .. } => {
                    let heading_matches = self.search.matches(label);
                    let mut group = vec![self.all_items[idx].clone()];
                    let mut kept_destination = false;
                    let mut end = idx + 1;
                    while let Some(item) = self.all_items.get(end).filter(|item| item.is_branch()) {
                        match item {
                            PaneMoveTargetItem::Destination(entry)
                                if heading_matches
                                    || self.search.matches(&pane_move_search_text(entry)) =>
                            {
                                kept_destination = true;
                                group.push(item.clone());
                            }
                            PaneMoveTargetItem::Here(_) => group.push(item.clone()),
                            _ => {}
                        }
                        end += 1;
                    }
                    if kept_destination {
                        groups.push(group);
                    }
                    idx = end;
                }
                PaneMoveTargetItem::Destination(entry) => {
                    if self.search.matches(&pane_move_search_text(entry)) {
                        groups.push(vec![self.all_items[idx].clone()]);
                    }
                    idx += 1;
                }
                PaneMoveTargetItem::Here(_) | PaneMoveTargetItem::Gap => idx += 1,
            }
        }
        let mut items = Vec::new();
        for (n, mut group) in groups.into_iter().enumerate() {
            if n > 0 {
                items.push(PaneMoveTargetItem::Gap);
            }
            items.append(&mut group);
        }
        self.items = items;
        self.select_first_destination();
    }

    pub fn destination_at(&self, idx: usize) -> Option<&PaneMoveTargetEntry> {
        match self.items.get(idx) {
            Some(PaneMoveTargetItem::Destination(entry)) => Some(entry),
            _ => None,
        }
    }

    pub fn selected_destination(&self) -> Option<&PaneMoveTargetEntry> {
        self.destination_at(self.list.selected)
    }

    /// Moves the selection to the next destination, stepping over headings and
    /// staying put when there is none rather than landing on a heading.
    pub fn select_next(&mut self) {
        if let Some(idx) = (self.list.selected.saturating_add(1)..self.items.len())
            .find(|idx| self.destination_at(*idx).is_some())
        {
            self.list.select(idx);
        }
    }

    /// Mirror of [`Self::select_next`] in the other direction.
    pub fn select_prev(&mut self) {
        if let Some(idx) = (0..self.list.selected)
            .rev()
            .find(|idx| self.destination_at(*idx).is_some())
        {
            self.list.select(idx);
        }
    }

    /// Points the selection at `idx` only when it holds a destination, so
    /// pointer hover cannot park the selection on a heading.
    pub fn select_destination(&mut self, idx: usize) -> bool {
        if self.destination_at(idx).is_none() {
            return false;
        }
        self.list.select(idx);
        true
    }

    /// Keep the selection in a `visible`-row window, with the heading above it
    /// when the two fit, so the first destination of a space never scrolls in
    /// without the space's name.
    pub fn reveal(&mut self, visible: usize) {
        let len = self.items.len();
        let heading = (0..self.list.selected)
            .rev()
            .take_while(|idx| !matches!(self.items.get(*idx), Some(PaneMoveTargetItem::Gap)))
            .find(|idx| {
                matches!(
                    self.items.get(*idx),
                    Some(PaneMoveTargetItem::SpaceHeading { .. })
                )
            });
        match heading {
            Some(heading) if self.list.selected - heading < visible => {
                self.list.reveal_with_context(heading, visible, len);
            }
            _ => self.list.reveal(visible, len),
        }
    }
}

/// What a destination is found by: its space, its row text and its panes.
fn pane_move_search_text(entry: &PaneMoveTargetEntry) -> String {
    format!(
        "{} {} {}",
        entry.facts.space,
        crate::ui::pane_move_target_row_label(entry),
        entry.facts.pane_names.join(" ")
    )
}

#[derive(Debug, Clone)]
pub struct ThemeRuntimeConfig {
    pub manual_name: String,
    pub dark_name: String,
    pub light_name: String,
    pub auto_switch: bool,
    pub custom: Option<crate::config::CustomThemeColors>,
    pub legacy_accent: Option<String>,
}

pub struct SettingsState {
    /// Which section tab is active.
    pub section: SettingsSection,
    /// Selected item index within the current section.
    pub list: ListCursor,
    /// The palette before opening settings (for cancel/restore).
    pub original_palette: Option<Palette>,
    /// The theme name before opening settings.
    pub original_theme: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceDropTarget {
    Before(usize),
    End,
}

pub(crate) enum DragTarget {
    WorkspaceReorder {
        source_id: crate::app::InputSourceId,
        source_ws_idx: usize,
        drop_target: Option<WorkspaceDropTarget>,
    },
    TabReorder {
        source_id: crate::app::InputSourceId,
        ws_idx: usize,
        source_tab_idx: usize,
        insert_idx: Option<usize>,
    },
    WorkspaceListScrollbar {
        grab_row_offset: u16,
    },
    AgentPanelScrollbar {
        grab_row_offset: u16,
    },
    PaneSplit {
        path: Vec<bool>,
        direction: Direction,
        area: Rect,
        grab_offset: u16,
    },
    PaneScrollbar {
        pane_id: crate::layout::PaneId,
        grab_row_offset: u16,
    },
    ReleaseNotesScrollbar {
        grab_row_offset: u16,
    },
    ProductAnnouncementScrollbar {
        grab_row_offset: u16,
    },
    KeybindHelpScrollbar {
        grab_row_offset: u16,
    },
    SidebarDivider,
    SidebarSectionDivider,
}

/// Active mouse drag on a split border or sidebar divider.
pub(crate) struct DragState {
    pub target: DragTarget,
}

pub(crate) struct WorkspacePressState {
    pub ws_idx: usize,
    pub start_col: u16,
    pub start_row: u16,
}

pub(crate) struct TabPressState {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub start_col: u16,
    pub start_row: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextMenuKind {
    Workspace {
        ws_idx: usize,
        pinned: bool,
    },
    GitWorkspace {
        ws_idx: usize,
        pinned: bool,
        is_linked_worktree: bool,
        has_worktree_children: bool,
        collapsed: bool,
    },
    Tab {
        ws_idx: usize,
        tab_idx: usize,
    },
    /// An agent row in the agent panel.
    Agent {
        ws_idx: usize,
        tab_idx: usize,
        pane_id: PaneId,
        pinned: bool,
    },
    Pane {
        ws_idx: usize,
        tab_idx: usize,
        pane_id: PaneId,
        source_pane_id: Option<PaneId>,
        has_manual_label: bool,
        right_click_passthrough: bool,
    },
}

/// Right-click context menu state.
pub struct ContextMenuState {
    pub kind: ContextMenuKind,
    pub x: u16,
    pub y: u16,
    pub list: ListCursor,
}

/// The pin item's label follows the state of the row it acts on.
fn pin_label(pinned: bool) -> &'static str {
    if pinned {
        "Unpin"
    } else {
        "Pin"
    }
}

impl ContextMenuState {
    pub fn items(&self) -> Vec<&'static str> {
        match self.kind {
            ContextMenuKind::Workspace { pinned, .. } => {
                vec![pin_label(pinned), "Rename", "Close"]
            }
            ContextMenuKind::GitWorkspace {
                pinned,
                is_linked_worktree: false,
                has_worktree_children: false,
                ..
            } => vec![
                pin_label(pinned),
                "Rename",
                "Close",
                "New worktree",
                "Open worktree...",
            ],
            ContextMenuKind::GitWorkspace {
                pinned,
                is_linked_worktree: true,
                ..
            } => vec![
                pin_label(pinned),
                "Rename",
                "Close",
                "Delete worktree checkout...",
            ],
            ContextMenuKind::GitWorkspace {
                pinned,
                is_linked_worktree: false,
                has_worktree_children: true,
                collapsed,
                ..
            } => vec![
                pin_label(pinned),
                "Rename",
                "Close group",
                "New worktree",
                "Open worktree...",
                if collapsed { "Expand" } else { "Collapse" },
            ],
            ContextMenuKind::Agent { pinned, .. } => vec![pin_label(pinned), "Rename pane"],
            ContextMenuKind::Tab { .. } => vec!["New tab", "Rename", "Close"],
            ContextMenuKind::Pane {
                source_pane_id,
                has_manual_label,
                right_click_passthrough,
                ..
            } => {
                let mut items = vec!["Rename pane"];
                if has_manual_label {
                    items.push("Clear pane name");
                }
                if source_pane_id.is_some() {
                    items.push("Swap with focused pane");
                }
                items.extend(["Split right", "Split down", "Zoom", "Sync input"]);
                items.push(if right_click_passthrough {
                    "Use Herdr right-click menu"
                } else {
                    "Send right-clicks to pane"
                });
                items.push("Close pane");
                items
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    NeedsAttention,
    Finished,
    UpdateInstalled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastTarget {
    pub workspace_id: String,
    pub pane_id: PaneId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastNotification {
    pub kind: ToastKind,
    pub title: String,
    pub context: String,
    pub position: Option<crate::config::ToastHerdrPosition>,
    pub target: Option<ToastTarget>,
    /// The pane a note about a pane action is about; with
    /// `[ui.toast.herdr] pane_feedback = "pane"` it is drawn there (fork
    /// issue 129). `None` for every other toast.
    pub anchor_pane: Option<PaneId>,
}

/// Upper bound on retained notification log entries; older entries are
/// evicted. Deliberately a constant, not config.
pub const NOTIFICATION_LOG_CAPACITY: usize = 100;

/// One entry in the server-owned notification log. Mirrors the toast that was
/// shown; `target` reuses the toast's workspace/pane identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationEntry {
    pub id: u64,
    pub kind: ToastKind,
    pub title: String,
    pub context: String,
    pub target: Option<ToastTarget>,
    pub posted_at_unix: u64,
    /// Per-entry read flag: false when posted, set when the user activates
    /// the entry (or marks all read). Unread is counted from this, not from
    /// a high-water mark, so visiting one notification quiets exactly one.
    pub read: bool,
}

/// Server-owned bounded notification log with per-entry read flags. Every
/// herdr toast is appended here through `AppState::post_notification`, so the
/// log cannot disagree with the toasts the user actually saw. In-memory
/// only: empties on a cold server restart.
#[derive(Debug, Default)]
pub struct NotificationLog {
    entries: std::collections::VecDeque<NotificationEntry>,
    last_id: u64,
    /// Entries posted but not yet emitted as `notification.posted` events.
    /// Drained by the App layer, which owns the event hub.
    pending_events: Vec<NotificationEntry>,
}

impl NotificationLog {
    pub(crate) fn post(&mut self, toast: &ToastNotification, posted_at_unix: u64) -> u64 {
        self.last_id += 1;
        let entry = NotificationEntry {
            id: self.last_id,
            kind: toast.kind,
            title: toast.title.clone(),
            context: toast.context.clone(),
            target: toast.target.clone(),
            posted_at_unix,
            read: false,
        };
        self.entries.push_back(entry.clone());
        while self.entries.len() > NOTIFICATION_LOG_CAPACITY {
            self.entries.pop_front();
        }
        self.pending_events.push(entry);
        self.last_id
    }

    pub fn entries_newest_first(&self) -> impl Iterator<Item = &NotificationEntry> {
        self.entries.iter().rev()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries not yet read. Evicted entries cannot be unread.
    pub fn unread_count(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.read).count()
    }

    /// Mark one entry read. Idempotent; returns whether the flag changed
    /// (false for unknown ids and already-read entries).
    pub fn mark_read(&mut self, id: u64) -> bool {
        self.entries
            .iter_mut()
            .find(|entry| entry.id == id && !entry.read)
            .map(|entry| {
                entry.read = true;
            })
            .is_some()
    }

    /// Mark every entry read, keeping the log. Idempotent; returns whether
    /// any entry changed.
    pub fn mark_all_seen(&mut self) -> bool {
        let mut changed = false;
        for entry in self.entries.iter_mut() {
            changed |= !entry.read;
            entry.read = true;
        }
        changed
    }

    /// Empty the log. `last_id` stays monotonic so future entries never reuse
    /// an id; an empty log has zero unread. Returns how many entries were
    /// removed.
    pub(crate) fn clear(&mut self) -> usize {
        let removed = self.entries.len();
        self.entries.clear();
        self.pending_events.clear();
        removed
    }

    pub(crate) fn take_pending_events(&mut self) -> Vec<NotificationEntry> {
        std::mem::take(&mut self.pending_events)
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Footer buttons of the notification center panel, in render order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationCenterButton {
    MarkRead,
    Clear,
    Close,
}

/// TUI-only state for the notification center dropdown panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationCenterState {
    /// Cursor into the newest-first entry list.
    pub list: ListCursor,
    /// Which footer button the pointer is over, if any.
    pub hovered_button: Option<NotificationCenterButton>,
}

/// Footer buttons of the pane todo panel, in render order. `Add` leads so it
/// keeps the same position whether or not the pane holds todos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneTodoPanelButton {
    Add,
    Toggle,
    Go,
    ClearDone,
    Close,
}

/// TUI-only state for the pane todo panel. The todos themselves are
/// server-owned on `TerminalState`; only the cursor and the hover live here,
/// and neither is persisted or exposed over the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTodoPanelState {
    /// Pane whose todos the panel is showing.
    pub pane_id: PaneId,
    /// Cursor into the pane's presentation-order list.
    pub list: ListCursor,
    /// Which footer button the pointer is over, if any.
    pub hovered_button: Option<PaneTodoPanelButton>,
}

/// The edit modal's link control. `Keep` leaves whatever the todo already has
/// — including a dead link, which the store preserves — untouched; the other
/// two are explicit choices. These map exactly onto `todo.update`'s
/// `link_pane_id` / `clear_link` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneTodoEditLink {
    Keep,
    Clear,
    Set(PaneId),
}

/// The surface an edit modal was opened over, carried by the modal for the
/// duration.
///
/// One value rather than a field per surface: an edit is opened over exactly
/// one of them, and modelling it that way keeps "which one do we go back to"
/// a match rather than a pair of `Option`s whose combinations mean nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuspendedTodoSurface {
    Panel(PaneTodoPanelState),
    Board(TodoBoardState),
}

impl SuspendedTodoSurface {
    /// The overlay to reopen when the modal closes.
    fn into_overlay(self) -> Overlay {
        match self {
            Self::Panel(panel) => Overlay::PaneTodos(panel),
            Self::Board(board) => Overlay::TodoBoard(board),
        }
    }

    /// The panel, when it is a panel — the board has no single pane to answer
    /// for.
    fn panel(&self) -> Option<&PaneTodoPanelState> {
        match self {
            Self::Panel(panel) => Some(panel),
            Self::Board(_) => None,
        }
    }
}

/// TUI-only state for the pane todo edit modal: the in-progress buffer until
/// save. Deliberately its own `text` rather than the shared `name_input`, so a
/// cancelled rename can never leak into a todo save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTodoEditState {
    pub pane_id: PaneId,
    /// `None` while composing a brand-new todo.
    pub todo_id: Option<u64>,
    /// A cursor-bearing field rather than a bare `String`: every readline
    /// motion needs an insertion point to move, and the store's limit is
    /// enforced here so the modal cannot compose a todo the server rejects.
    pub text: crate::ui::text_field::TextField,
    pub priority: crate::terminal::todo::TodoPriority,
    pub link: PaneTodoEditLink,
    /// Only meaningful while editing an existing todo; a todo being composed
    /// is never already done, and `todo.add` has no `done` to carry it.
    pub done: bool,
    /// The todo surface this edit was opened from, suspended here for the
    /// duration and handed back when the modal closes. `None` when the modal
    /// was opened straight from a keybinding, which is what "return to the
    /// terminal instead" is read from.
    pub suspended: Option<SuspendedTodoSurface>,
}

/// Footer buttons of the todo board, in render order. `open` leads because
/// focusing a todo's pane is the board's reason to exist over `herdr todo
/// list --all`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoBoardButton {
    Open,
    Toggle,
    Go,
    ClearDone,
    Close,
}

/// One row of the board. Pane headings are rendered but never selectable, so
/// render and selection read the same list instead of deriving headings twice
/// — the shape [`PaneMoveTargetPickerState`] already uses for its space
/// headings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoBoardItem {
    PaneHeading {
        /// The owning space's display name. The heading leads with it because
        /// the board is read across spaces: where the work lives is the first
        /// thing you are deciding between.
        space: String,
        label: String,
    },
    Todo {
        pane_id: PaneId,
        todo_id: u64,
    },
    /// The blank row separating one group from the next.
    ///
    /// An item rather than an offset the renderer adds, because the scroll
    /// window, the selection and the mouse hit-test all rest on list row *n*
    /// being item *n*. A renderer-side offset would hold that only in the
    /// renderer, and a click below the first gap would land on the wrong todo.
    ///
    /// Nothing has to opt out of selecting it: everything that asks whether a
    /// row is a destination goes through [`TodoBoardState::todo_at`], which
    /// answers `None` for anything that is not a todo.
    GroupGap,
}

/// TUI-only state for the session todo board. The todos are server-owned on
/// `TerminalState`; the board holds a projection of them plus a cursor, and
/// neither is persisted or exposed over the API.
///
/// The projection is built when the board opens and rebuilt after every
/// mutation rather than per frame, so a session with many panes pays for it
/// once per action instead of once per render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoBoardState {
    /// Every group of the projection. The list is `items`, this filtered by
    /// the search; the board is sized from this, so a query does not resize
    /// it.
    pub all_items: Vec<TodoBoardItem>,
    pub items: Vec<TodoBoardItem>,
    pub list: ListCursor,
    pub hovered_button: Option<TodoBoardButton>,
    pub search: crate::ui::overlay::ListSearch,
}

impl TodoBoardState {
    /// Builds a board selecting the first todo, stepping over the heading that
    /// always precedes it.
    pub fn new(items: Vec<TodoBoardItem>) -> Self {
        let selected = items
            .iter()
            .position(|item| matches!(item, TodoBoardItem::Todo { .. }))
            .unwrap_or(0);
        Self {
            all_items: items.clone(),
            items,
            list: ListCursor::new(selected),
            hovered_button: None,
            search: crate::ui::overlay::ListSearch::default(),
        }
    }

    pub fn todo_at(&self, idx: usize) -> Option<(PaneId, u64)> {
        match self.items.get(idx) {
            Some(TodoBoardItem::Todo { pane_id, todo_id }) => Some((*pane_id, *todo_id)),
            _ => None,
        }
    }

    pub fn selected_todo(&self) -> Option<(PaneId, u64)> {
        self.todo_at(self.list.selected)
    }

    /// Points the selection at `idx` only when it holds a todo, so a click or
    /// a hover cannot park the selection on a heading.
    pub fn select_todo(&mut self, idx: usize) -> bool {
        if self.todo_at(idx).is_none() {
            return false;
        }
        self.list.select(idx);
        true
    }

    /// The nearest todo at or past `target`, searching in the direction of
    /// travel first and falling back the other way at either end — so no
    /// movement can park on a heading, and a jump to the first or last item
    /// lands on a todo rather than on the heading above it.
    /// The heading that labels the item at `idx` — the nearest one at or
    /// before it. Scrolling keeps this on screen with the selection, because a
    /// selected row whose heading has scrolled away does not say which pane it
    /// belongs to.
    pub fn group_heading_index(&self, idx: usize) -> usize {
        (0..=idx.min(self.items.len().saturating_sub(1)))
            .rev()
            .find(|i| matches!(self.items.get(*i), Some(TodoBoardItem::PaneHeading { .. })))
            .unwrap_or(0)
    }

    pub fn nearest_todo(&self, target: usize, forward: bool) -> Option<usize> {
        let ahead: Option<usize> = if forward {
            (target..self.items.len()).find(|idx| self.todo_at(*idx).is_some())
        } else {
            (0..=target.min(self.items.len().saturating_sub(1)))
                .rev()
                .find(|idx| self.todo_at(*idx).is_some())
        };
        ahead.or_else(|| {
            if forward {
                (0..target.min(self.items.len()))
                    .rev()
                    .find(|idx| self.todo_at(*idx).is_some())
            } else {
                (target.saturating_add(1)..self.items.len())
                    .find(|idx| self.todo_at(*idx).is_some())
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAgentNotification {
    pub pane_id: PaneId,
    pub workspace_id: String,
    pub agent_label: String,
    pub known_agent: Option<crate::detect::Agent>,
    pub kind: ToastKind,
    pub state: AgentState,
    pub deadline: std::time::Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentNotificationDelivery {
    pub pane_id: PaneId,
    pub workspace_id: String,
    pub agent_label: String,
    pub known_agent: Option<crate::detect::Agent>,
    pub kind: ToastKind,
    pub toast: Option<ToastNotification>,
    pub client_notification: Option<ToastNotification>,
    pub sound: Option<crate::sound::Sound>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyFeedback {
    pub message: String,
    /// The pane the copied text came from, when known (fork issue 129).
    pub source_pane: Option<PaneId>,
}

/// A clipboard write herdr's own UI asked for, and the pane the text came
/// from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardWriteRequest {
    pub content: Vec<u8>,
    pub source_pane: Option<PaneId>,
}

pub struct ReleaseNotesState {
    pub version: String,
    pub body: String,
    pub scroll: u16,
    pub preview: bool,
}

pub struct ProductAnnouncementState {
    pub version: String,
    pub id: String,
    pub title: String,
    pub body: String,
    pub scroll: u16,
    pub preview: bool,
}

pub struct KeybindHelpState {
    pub scroll: u16,
    pub query: crate::ui::text_field::TextField,
    pub search_focused: bool,
}

impl Default for KeybindHelpState {
    fn default() -> Self {
        Self {
            scroll: 0,
            query: search_query_field(),
            search_focused: false,
        }
    }
}

/// An empty overlay search box.
pub(crate) fn search_query_field() -> crate::ui::text_field::TextField {
    crate::ui::text_field::TextField::new(SEARCH_QUERY_MAX_CHARS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarWidthSource {
    ConfigDefault,
    Persisted,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneFocusTarget {
    pub workspace_id: String,
    pub pane_id: PaneId,
}

/// All application state — pure data, no channels or async runtime.
/// Testable without PTYs or a tokio runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabBarStatusSegment {
    Zoom,
    Text(Option<String>),
}

// ---------------------------------------------------------------------------
// The open overlay
// ---------------------------------------------------------------------------

/// Declares the one open overlay, and the accessors every call site reaches it
/// through.
///
/// One list is the point: a variant, the state it carries, the mode it puts the
/// app in, and whether its keys are commands rather than free text. Adding an
/// overlay means adding a line here, and the behaviour that used to be restated
/// in a separate allowlist per concern comes with it.
macro_rules! overlays {
    ($(
        $(#[$meta:meta])*
        $variant:ident($state:ty) => mode $mode:ident, ascii $ascii:literal,
            $get:ident / $get_mut:ident / $take:ident;
    )+) => {
        /// The overlay that is open, if any. One value rather than a mode plus
        /// ten-plus parallel `Option<XState>` fields paired by convention: it
        /// is not representable for the active mode to name one overlay while
        /// a different overlay's state is present, nor for two to be present
        /// at once.
        pub enum Overlay {
            $( $(#[$meta])* $variant($state), )+
        }

        /// An overlay with its state left out, so the set can be enumerated.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum OverlayKind {
            $( $variant, )+
        }

        impl Overlay {
            /// The mode this overlay puts the app in. Input dispatch stays
            /// keyed on `Mode`; the variant supplies it rather than replacing
            /// it.
            pub(crate) fn mode(&self) -> Mode {
                match self {
                    $( Self::$variant(_) => Mode::$mode, )+
                }
            }

            pub(crate) fn kind(&self) -> OverlayKind {
                match self {
                    $( Self::$variant(_) => OverlayKind::$variant, )+
                }
            }
        }

        impl OverlayKind {
            /// Every overlay, for the guard tests that must not be allowed to
            /// miss one.
            pub(crate) const ALL: &'static [Self] = &[$( Self::$variant, )+];

            pub(crate) fn mode(self) -> Mode {
                match self {
                    $( Self::$variant => Mode::$mode, )+
                }
            }

            /// Whether keys in this overlay are commands and navigation (an
            /// ASCII input source is wanted) rather than free text. Declared
            /// with the variant, so a new overlay cannot fall off a list it
            /// never knew about.
            pub(crate) fn wants_ascii_input(self) -> bool {
                match self {
                    $( Self::$variant => $ascii, )+
                }
            }
        }

        impl AppState {
            $(
                pub(crate) fn $get(&self) -> Option<&$state> {
                    match self.overlay.as_ref() {
                        Some(Overlay::$variant(state)) => Some(state),
                        _ => None,
                    }
                }

                // Generated for every overlay so the accessor set is uniform;
                // an overlay with nothing to edit in place (display panes)
                // never calls it.
                #[allow(dead_code)]
                pub(crate) fn $get_mut(&mut self) -> Option<&mut $state> {
                    match self.overlay.as_mut() {
                        Some(Overlay::$variant(state)) => Some(state),
                        _ => None,
                    }
                }

                /// Take this overlay's state, closing it. Leaves a different
                /// open overlay alone.
                // Generated for every overlay so the accessor set is uniform;
                // only the ones whose close path hands their state on — the
                // navigator, the todo panel, the todo edit — call it today.
                #[allow(dead_code)]
                pub(crate) fn $take(&mut self) -> Option<$state> {
                    match self.overlay.take() {
                        Some(Overlay::$variant(state)) => Some(state),
                        other => {
                            self.overlay = other;
                            None
                        }
                    }
                }
            )+
        }
    };
}

overlays! {
    Settings(SettingsState) => mode Settings, ascii false,
        settings / settings_mut / take_settings;
    GlobalMenu(ListCursor) => mode GlobalMenu, ascii true,
        global_menu / global_menu_mut / take_global_menu;
    KeybindHelp(KeybindHelpState) => mode KeybindHelp, ascii true,
        keybind_help / keybind_help_mut / take_keybind_help;
    Navigator(NavigatorState) => mode Navigator, ascii true,
        navigator / navigator_mut / take_navigator;
    ContextMenu(ContextMenuState) => mode ContextMenu, ascii true,
        context_menu / context_menu_mut / take_context_menu;
    NotificationCenter(NotificationCenterState) => mode NotificationCenter, ascii true,
        notification_center / notification_center_mut / take_notification_center;
    PaneTodos(PaneTodoPanelState) => mode PaneTodos, ascii true,
        pane_todos / pane_todos_mut / take_pane_todos;
    PaneTodoEdit(PaneTodoEditState) => mode PaneTodoEdit, ascii false,
        pane_todo_edit / pane_todo_edit_mut / take_pane_todo_edit;
    ReleaseNotes(ReleaseNotesState) => mode ReleaseNotes, ascii false,
        release_notes / release_notes_mut / take_release_notes;
    ProductAnnouncement(ProductAnnouncementState) => mode ProductAnnouncement, ascii false,
        product_announcement / product_announcement_mut / take_product_announcement;
    NewLinkedWorktree(WorktreeCreateState) => mode NewLinkedWorktree, ascii false,
        worktree_create / worktree_create_mut / take_worktree_create;
    OpenExistingWorktree(WorktreeOpenState) => mode OpenExistingWorktree, ascii false,
        worktree_open / worktree_open_mut / take_worktree_open;
    ConfirmRemoveWorktree(WorktreeRemoveState) => mode ConfirmRemoveWorktree, ascii true,
        worktree_remove / worktree_remove_mut / take_worktree_remove;
    PaneMoveTargetPicker(PaneMoveTargetPickerState) => mode PaneMoveTargetPicker, ascii true,
        pane_move_target_picker / pane_move_target_picker_mut / take_pane_move_target_picker;
    TodoBoard(TodoBoardState) => mode TodoBoard, ascii true,
        todo_board / todo_board_mut / take_todo_board;
    DisplayPanes(DisplayPanesState) => mode DisplayPanes, ascii true,
        display_panes / display_panes_mut / take_display_panes;
}

impl AppState {
    /// Open an overlay, putting the app in its mode. Whatever was open closes:
    /// one overlay at a time is the invariant.
    pub(crate) fn open_overlay(&mut self, overlay: Overlay) {
        self.mode = overlay.mode();
        self.overlay = Some(overlay);
    }

    /// Replace the open overlay without touching the mode, for the callers
    /// that set the mode themselves on the next line. Mode and overlay still
    /// cannot disagree — [`AppState::assert_invariants_for_test`] checks it.
    pub(crate) fn set_overlay(&mut self, overlay: Overlay) {
        self.overlay = Some(overlay);
    }

    /// Close this overlay if it is the one that is open. Leaves the mode to the
    /// caller, which knows where it is going back to.
    pub(crate) fn close_overlay(&mut self, kind: OverlayKind) {
        if self
            .overlay
            .as_ref()
            .is_some_and(|open| open.kind() == kind)
        {
            self.overlay = None;
        }
    }

    /// Close whatever is open.
    pub(crate) fn close_any_overlay(&mut self) {
        self.overlay = None;
    }

    /// Which overlay is open, if any.
    #[cfg(test)]
    pub(crate) fn open_overlay_kind(&self) -> Option<OverlayKind> {
        self.overlay.as_ref().map(Overlay::kind)
    }

    /// The keybind-help filter, empty when the panel is closed. Read by the
    /// line builder, which also runs from the help-coverage guard test.
    pub(crate) fn keybind_help_query(&self) -> &str {
        self.keybind_help()
            .map(|help| help.query.text())
            .unwrap_or_default()
    }

    /// The settings section on show, defaulting to the first tab when the
    /// panel is closed — the geometry helpers are asked for a size before the
    /// panel is drawn.
    pub(crate) fn settings_section(&self) -> SettingsSection {
        self.settings()
            .map(|settings| settings.section)
            .unwrap_or(SettingsSection::Theme)
    }

    /// The settings list cursor, a copy: the panel's sections read and write
    /// it around calls that also want the rest of `AppState`.
    pub(crate) fn settings_list(&self) -> ListCursor {
        self.settings()
            .map(|settings| settings.list)
            .unwrap_or_default()
    }

    pub(crate) fn set_settings_list(&mut self, list: ListCursor) {
        if let Some(settings) = self.settings_mut() {
            settings.list = list;
        }
    }

    pub(crate) fn set_settings_selected(&mut self, selected: usize) {
        if let Some(settings) = self.settings_mut() {
            settings.list.selected = selected;
        }
    }

    pub(crate) fn set_settings_section(&mut self, section: SettingsSection) {
        if let Some(settings) = self.settings_mut() {
            settings.section = section;
        }
    }

    /// The navigator's search text, empty when it is closed.
    pub(crate) fn navigator_query(&self) -> &str {
        self.navigator()
            .map(|navigator| navigator.search.query.text())
            .unwrap_or_default()
    }

    pub(crate) fn navigator_state_filter(&self) -> Option<NavigatorStateFilter> {
        self.navigator()
            .and_then(|navigator| navigator.state_filter)
    }

    pub(crate) fn navigator_purpose(&self) -> NavigatorPurpose {
        self.navigator()
            .map(|navigator| navigator.purpose)
            .unwrap_or_default()
    }

    pub(crate) fn navigator_selected(&self) -> usize {
        self.navigator()
            .map(|navigator| navigator.selected)
            .unwrap_or(0)
    }

    pub(crate) fn navigator_scroll(&self) -> usize {
        self.navigator()
            .map(|navigator| navigator.scroll)
            .unwrap_or(0)
    }

    pub(crate) fn navigator_workspace_expanded(&self, workspace_id: &str) -> bool {
        self.navigator()
            .is_some_and(|navigator| navigator.expanded_workspaces.contains(workspace_id))
    }

    pub(crate) fn set_navigator_selected(&mut self, selected: usize) {
        if let Some(navigator) = self.navigator_mut() {
            navigator.selected = selected;
        }
    }

    /// Expand or collapse a workspace in the navigator's tree.
    pub(crate) fn toggle_navigator_workspace_expanded(&mut self, workspace_id: String) {
        if let Some(navigator) = self.navigator_mut() {
            if !navigator.expanded_workspaces.remove(&workspace_id) {
                navigator.expanded_workspaces.insert(workspace_id);
            }
        }
    }

    /// Replace the navigator's search text, for tests that drive it directly.
    #[cfg(test)]
    pub(crate) fn set_keybind_help_query(&mut self, query: &str) {
        if let Some(help) = self.keybind_help_mut() {
            help.query = crate::ui::text_field::TextField::from_text(query, SEARCH_QUERY_MAX_CHARS);
        }
    }

    #[cfg(test)]
    pub(crate) fn set_navigator_query(&mut self, query: &str) {
        if let Some(navigator) = self.navigator_mut() {
            navigator.search.query =
                crate::ui::text_field::TextField::from_text(query, SEARCH_QUERY_MAX_CHARS);
        }
    }

    #[cfg(test)]
    pub(crate) fn navigator_expanded_count(&self) -> usize {
        self.navigator()
            .map(|navigator| navigator.expanded_workspaces.len())
            .unwrap_or(0)
    }

    pub(crate) fn set_navigator_state_filter(&mut self, filter: Option<NavigatorStateFilter>) {
        if let Some(navigator) = self.navigator_mut() {
            navigator.state_filter = filter;
        }
    }

    #[cfg(test)]
    pub(crate) fn expand_navigator_workspace(&mut self, workspace_id: String) {
        if let Some(navigator) = self.navigator_mut() {
            navigator.expanded_workspaces.insert(workspace_id);
        }
    }

    pub(crate) fn navigator_search_focused(&self) -> bool {
        self.navigator()
            .is_some_and(|navigator| navigator.search.focused)
    }

    #[cfg(test)]
    pub(crate) fn set_navigator_search_focused(&mut self, focused: bool) {
        if let Some(navigator) = self.navigator_mut() {
            navigator.search.focused = focused;
        }
    }

    /// Run `f` on an overlay search box's text, if that overlay is open.
    /// The field is moved out for the duration so `f` can drive the shared
    /// editing set against it while the rest of `AppState` stays reachable.
    pub(crate) fn edit_keybind_help_query<T>(
        &mut self,
        f: impl FnOnce(&mut crate::ui::text_field::TextField) -> T,
    ) -> Option<T> {
        let help = self.keybind_help_mut()?;
        let mut query = std::mem::replace(&mut help.query, search_query_field());
        let out = f(&mut query);
        if let Some(help) = self.keybind_help_mut() {
            help.query = query;
        }
        Some(out)
    }

    pub(crate) fn clear_navigator_query(&mut self) {
        if let Some(navigator) = self.navigator_mut() {
            navigator.search.query.clear();
        }
    }

    pub(crate) fn global_menu_selected(&self) -> usize {
        self.global_menu().map(|menu| menu.selected).unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn keybind_help_scroll(&self) -> u16 {
        self.keybind_help().map(|help| help.scroll).unwrap_or(0)
    }

    pub(crate) fn set_keybind_help_scroll(&mut self, scroll: u16) {
        if let Some(help) = self.keybind_help_mut() {
            help.scroll = scroll;
        }
    }

    pub(crate) fn keybind_help_search_focused(&self) -> bool {
        self.keybind_help().is_some_and(|help| help.search_focused)
    }

    pub(crate) fn set_keybind_help_search_focused(&mut self, focused: bool) {
        if let Some(help) = self.keybind_help_mut() {
            help.search_focused = focused;
        }
    }

    pub(crate) fn clear_keybind_help_query(&mut self) {
        if let Some(help) = self.keybind_help_mut() {
            help.query.clear();
        }
    }

    pub(crate) fn set_navigator_scroll(&mut self, scroll: usize) {
        if let Some(navigator) = self.navigator_mut() {
            navigator.scroll = scroll;
        }
    }

    /// The todo being composed, whether its modal is the open overlay or it is
    /// suspended behind the link picker.
    pub(crate) fn editing_pane_todo(&self) -> Option<&PaneTodoEditState> {
        self.pane_todo_edit().or_else(|| {
            self.navigator()
                .and_then(|navigator| navigator.suspended_pane_todo_edit.as_ref())
        })
    }

    #[cfg(test)]
    pub(crate) fn editing_pane_todo_mut(&mut self) -> Option<&mut PaneTodoEditState> {
        match self.overlay.as_mut() {
            Some(Overlay::PaneTodoEdit(edit)) => Some(edit),
            Some(Overlay::Navigator(navigator)) => navigator.suspended_pane_todo_edit.as_mut(),
            _ => None,
        }
    }
}

pub struct AppState {
    pub terminals:
        std::collections::HashMap<crate::terminal::TerminalId, crate::terminal::TerminalState>,
    /// Terminal ids whose size is currently owned by a direct attach client.
    pub direct_attach_resize_locks: std::collections::HashSet<crate::terminal::TerminalId>,
    pub(crate) pane_id_aliases: std::collections::HashMap<u32, PaneId>,
    pub(crate) public_pane_id_aliases: std::collections::HashMap<String, PaneId>,
    pub workspaces: Vec<Workspace>,
    pub active: Option<usize>,
    pub(crate) previous_pane_focus: Option<PaneFocusTarget>,
    pub selected: usize,
    pub mode: Mode,
    /// Stable workspace identity captured when the close confirmation opens.
    pub(crate) confirm_close_workspace_id: Option<String>,
    pub should_quit: bool,
    /// In monolithic --no-session mode, detach exits the app because there is no server to detach from.
    pub detach_exits: bool,
    /// Set when the current client should detach from the persistent session.
    /// The server's event loop checks this and handles client detach.
    pub detach_requested: bool,
    pub request_new_workspace: bool,
    pub request_new_tab: bool,
    pub request_new_linked_worktree: Option<usize>,
    pub request_open_existing_worktree: Option<usize>,
    pub request_new_workspace_cwd: Option<std::path::PathBuf>,
    pub request_remove_linked_worktree: Option<usize>,
    pub request_submit_worktree_create: bool,
    pub request_submit_worktree_open: bool,
    pub request_submit_worktree_remove: bool,
    pub request_reload_config: bool,
    /// Set when the headless server should ask attached clients to reload
    /// their client-local sound config from disk.
    pub request_client_config_reload: bool,
    /// Set when UI interaction requested a clipboard write that must be
    /// handled by the outer App/event loop instead of directly from AppState.
    pub request_clipboard_write: Option<ClipboardWriteRequest>,
    /// Sends queued by the alt-screen scroll passthrough mode; the App layer
    /// encodes them against the pinned pane's terminal state and sends them,
    /// keeping the PTY effect out of AppState.
    pub pending_app_scroll_sends: Vec<AppScrollSend>,
    pub creating_new_tab: bool,
    pub requested_new_tab_name: Option<String>,
    pub pending_workspace_create_cwd: Option<std::path::PathBuf>,
    pub rename_pane_target: Option<PaneId>,
    /// Pane whose close is waiting on the confirmation modal because it still
    /// has outstanding todos. Doubles as the "the user said yes" token: the
    /// close path consumes it.
    pub confirm_close_pane: Option<PaneId>,
    /// Pane whose respawn is waiting on the confirmation modal because it would
    /// kill live work. Same token semantics as `confirm_close_pane`, and
    /// mutually exclusive with it so answering one prompt can never perform the
    /// other action.
    pub confirm_respawn_pane: Option<PaneId>,
    /// The one open overlay. See [`Overlay`].
    pub(crate) overlay: Option<Overlay>,
    pub worktree_directory: std::path::PathBuf,
    pub collapsed_space_keys: std::collections::HashSet<String>,
    pub request_complete_onboarding: bool,
    pub name_input: crate::ui::text_field::TextField,
    pub name_input_replace_on_type: bool,
    pub copy_mode: Option<CopyModeState>,
    pub app_scroll: Option<AppScrollState>,
    pub workspace_scroll: usize,
    pub agent_panel_scroll: usize,
    /// Last workspace/agent identity the sidebar follow saw; a change
    /// re-engages the follow after manual scrolling disengaged it.
    pub sidebar_followed_workspace: Option<String>,
    pub sidebar_followed_agent: Option<(String, PaneId)>,
    /// While true, compute_view keeps the active workspace / focused agent
    /// entry visible even as the lists reorder (priority re-sorts, entries
    /// added or removed). Manual scrolling disengages; the next focus change
    /// re-engages. Mirrors `tab_scroll_follow_active`.
    pub workspace_list_follow_active: bool,
    pub agent_panel_follow_active: bool,
    pub tab_scroll: usize,
    pub tab_scroll_follow_active: bool,
    pub mobile_switcher_scroll: usize,
    // View geometry (computed before render, consumed by render + mouse)
    pub view: ViewState,
    pub(crate) drag: Option<DragState>,
    /// Until when the resize size labels stay after the last resize step
    /// (fork issue 122). Presentation only; see `display_panes`.
    pub(crate) resize_labels_until: Option<std::time::Instant>,
    /// The lingering labels are the window-resize view (fork issue 138):
    /// they also show the window summary bar and sidebar section sizes.
    pub(crate) resize_labels_window: bool,
    pub(crate) workspace_presses:
        std::collections::HashMap<crate::app::InputSourceId, WorkspacePressState>,
    pub(crate) tab_presses: std::collections::HashMap<crate::app::InputSourceId, TabPressState>,
    pub selection: Option<Selection>,
    pub selection_autoscroll: Option<SelectionAutoscroll>,
    // Notifications
    pub update_available: Option<String>,
    pub update_install_command: String,
    pub latest_release_notes_available: bool,
    pub update_dismissed: bool,
    pub config_diagnostic: Option<String>,
    pub toast: Option<ToastNotification>,
    /// Server-owned notification log fed by `post_notification`.
    pub notification_log: NotificationLog,
    pub pending_agent_notifications: std::collections::HashMap<PaneId, PendingAgentNotification>,
    pub copy_feedback: Option<CopyFeedback>,
    /// Last reported focus state for the outer terminal hosting herdr.
    /// None means unsupported or not yet reported, which preserves active-pane suppression.
    pub outer_terminal_focus: Option<bool>,
    // Config
    pub prefix_code: KeyCode,
    pub prefix_mods: KeyModifiers,
    /// Virtual terminal size (columns, rows) used when no client is attached.
    pub(crate) headless_size: (u16, u16),
    /// The size, (cols, rows), a server with no client attached lays new
    /// panes out at: one a live handoff carried, the remembered client size,
    /// or the configured headless size. `None` while a client is attached,
    /// when the view is the client's.
    pub(crate) detached_pane_size: Option<(u16, u16)>,
    /// The size, (cols, rows), of the last foreground client that was at least
    /// [`REMEMBERED_CLIENT_SIZE_FLOOR`]. The session file keeps it, so it
    /// outlives the client and the server.
    pub(crate) last_client_size: Option<(u16, u16)>,
    /// `server.remember_client_size`: whether the last client size is kept
    /// and used as the no-client size.
    pub(crate) remember_client_size: bool,
    pub default_sidebar_width: u16,
    pub sidebar_width: u16,
    pub sidebar_min_width: u16,
    pub sidebar_max_width: u16,
    pub mobile_width_threshold: u16,
    pub sidebar_width_source: SidebarWidthSource,
    pub sidebar_width_auto: bool,
    pub sidebar_collapsed: bool,
    pub sidebar_collapsed_mode: crate::config::SidebarCollapsedModeConfig,
    /// Ephemeral cursor for the "next layout" cycle action (not persisted).
    pub layout_cycle_index: usize,
    /// Ratio of sidebar height allocated to the workspaces section.
    pub sidebar_section_split: f32,
    pub agent_panel_sort: AgentPanelSort,
    pub status_indicators: crate::config::StatusIndicatorStyle,
    pub status_spinner: crate::config::StatusSpinnerConfig,
    /// Interval between working-spinner frames; see `ui.status_spinner_ms`.
    pub status_spinner_interval: std::time::Duration,
    /// How long the `prefix+i` labels and the resize labels stay up; see
    /// `ui.display_panes_ms`.
    pub display_panes_duration: std::time::Duration,
    /// Current working-spinner frame, stepped by `App::advance_spinner`.
    pub spinner_frame: u8,
    /// Transient session-wide projection override for the built-in Agents view.
    pub agent_view_override: Option<crate::api::schema::AgentViewSetParams>,
    pub sidebar_agents: crate::config::AgentsSidebarConfig,
    pub sidebar_spaces: crate::config::SpacesSidebarConfig,
    pub workspace_sort: WorkspaceSort,
    /// Whether priority re-sorts bubble (settle + stepped moves) or apply
    /// instantly. From `ui.sort_motion`.
    pub sort_motion_bubble: bool,
    /// Settle/step timing for bubble motion. From `ui.sort_motion_*_ms`.
    pub sort_motion_timing: crate::ui::list_motion::ListMotionTiming,
    /// Display-order motion state for the sidebar workspace list (unit keys:
    /// workspace public id or worktree-group key). Mutated only by the
    /// scheduled motion tick.
    pub workspace_list_motion: crate::ui::list_motion::ListMotion<String>,
    /// Display-order motion state for the agents panel (pane-id keys).
    pub agent_panel_motion: crate::ui::list_motion::ListMotion<PaneId>,
    /// Sidebar entry composition preset. From `ui.sidebar_style`.
    pub sidebar_style: crate::config::SidebarStyleConfig,
    /// Parsed `[ui.state_colors]` overrides; unset slots fall back to the
    /// theme palette in `state_icon_colors()`.
    pub state_color_overrides: StateColorOverrides,
    /// `[ui.state_symbols]` glyph overrides layered over `status_indicators`.
    pub state_symbol_overrides: StateSymbolOverrides,
    /// Where the notification center dropdown anchors (TUI presentation).
    pub notification_center_position: crate::config::NotificationCenterPositionConfig,
    pub next_agent_state_change_seq: u64,
    /// Capture mouse input for Herdr's own mouse UI. When false, Herdr only
    /// captures mouse while the focused pane app requests mouse reporting.
    pub mouse_capture: bool,
    pub copy_on_select: bool,
    pub right_click_passthrough_modifiers: Option<KeyModifiers>,
    pub right_click_passthrough: Option<RightClickPassthroughGesture>,
    pub redraw_on_focus_gained: bool,
    pub mouse_scroll_lines: usize,
    pub confirm_close: bool,
    pub prompt_new_tab_name: bool,
    pub prompt_new_workspace_name: bool,
    pub pane_borders: bool,
    pub pane_outer_borders: bool,
    pub pane_scrollbars: bool,
    pub pane_gaps: bool,
    pub show_agent_labels_on_pane_borders: bool,
    /// Draw the pane todo indicator on split pane top borders.
    pub show_pane_todo_indicator: bool,
    pub hide_tab_bar_when_single_tab: bool,
    pub tab_bar_position: TabBarPositionConfig,
    pub tab_bar_right: Vec<TabBarStatusSegment>,
    pub tab_bar_right_separator: String,
    pub show_workspace_numbers: bool,
    pub show_agent_numbers: bool,
    /// Show the server's short host name on the sidebar "SPACES" header row.
    pub show_host: bool,
    /// Short host name of the machine running the server, read once at
    /// construction. `None` when the OS lookup fails. Rendering is gated by
    /// `show_host`; reloads never re-read it because the host is stable.
    pub host_label: Option<String>,
    pub pane_history_persistence: bool,
    /// Expose the focused pane's cursor anchor to the outer terminal even when
    /// the pane requested `?25l`. See `[experimental] reveal_hidden_cursor_for_cjk_ime`.
    pub reveal_hidden_cursor_for_cjk_ime: bool,
    /// Restrict cursor reveal to focused panes whose detected agent matches
    /// one of these. When false, apply to any focused pane.
    pub cjk_ime_agent_filter_configured: bool,
    pub cjk_ime_agents: Vec<crate::detect::Agent>,
    /// DECSCUSR shape parameter (1–6) for the IME anchor cursor.
    pub cjk_ime_cursor_shape: u8,
    /// While prefix mode is active, switch the macOS host input source to an
    /// ASCII-capable layout so prefix commands register as ASCII even when a
    /// CJK IME is active. macOS only; a no-op elsewhere. See
    /// `[experimental] switch_ascii_input_source_in_prefix`.
    pub switch_ascii_input_source_in_prefix: bool,
    pub kitty_graphics_enabled: bool,
    pub default_shell: String,
    pub shell_mode: crate::config::ShellModeConfig,
    pub new_terminal_cwd: NewTerminalCwdConfig,
    pub pane_scrollback_limit_bytes: usize,
    #[allow(dead_code)] // kept for backward compat; palette.accent is the source of truth
    pub accent: Color,
    /// Override color for `show_workspace_numbers` labels; None uses the palette default.
    pub workspace_number_color: Option<Color>,
    /// Override color for `show_agent_numbers` labels; None uses the palette default.
    pub agent_number_color: Option<Color>,
    /// Leader glyph(s) before the workspace jump number in editorial style.
    pub workspace_number_prefix: String,
    /// Leader glyph(s) before the agent jump number in editorial style.
    pub agent_number_prefix: String,
    /// Override color for the focused (active) pane border; None uses the palette accent.
    pub pane_border_active_color: Option<Color>,
    /// Override color for unfocused (inactive) pane borders; None uses the palette default.
    pub pane_border_inactive_color: Option<Color>,
    /// Box-drawing weight for the focused pane border.
    pub pane_border_active_style: crate::config::PaneBorderActiveStyleConfig,
    /// Override color for the focused pane's border title; None follows
    /// `pane_border_active_color`, then the palette accent.
    pub pane_title_active_color: Option<Color>,
    /// Override color for unfocused panes' border titles; None follows
    /// `pane_border_inactive_color`, then the palette default.
    pub pane_title_inactive_color: Option<Color>,
    /// Override color for the pane todo indicator while todos are outstanding;
    /// None colors it by the highest outstanding priority.
    pub pane_todo_color: Option<Color>,
    /// Highlight pattern for the active space and agent in the sidebar,
    /// styled like the active pane border.
    pub sidebar_active_border: crate::config::SidebarActiveBorderConfig,
    pub sidebar_overflow: crate::config::SidebarOverflowConfig,
    /// Fog percent per row, nearest first (0: none), and the tint share.
    pub sidebar_fog: [u32; crate::config::SIDEBAR_FOG_ROWS],
    pub sidebar_fog_tint: u32,
    pub sidebar_fog_style: crate::config::SidebarFogStyle,
    pub sidebar_fade: [u32; crate::config::SIDEBAR_FOG_ROWS],
    /// Default background for the focused pane's cells; None keeps the
    /// terminal default. Only default-background cells are tinted.
    pub pane_active_bg: Option<Color>,
    /// Default background for unfocused panes' cells; None keeps the
    /// terminal default. Only default-background cells are tinted.
    pub pane_inactive_bg: Option<Color>,
    /// Dim unfocused pane content in all modes, not only in prefix/navigate.
    pub dim_inactive_panes: bool,
    pub inactive_pane_dim: u32,
    pub sound: SoundConfig,
    pub local_sound_playback: bool,
    pub toast_config: ToastConfig,
    pub keybinds: Keybinds,
    /// UI color palette — all sidebar/UI colors centralized for theming.
    pub palette: Palette,
    /// Currently applied theme name (for settings UI).
    pub theme_name: String,
    /// Runtime theme configuration used to resolve manual and auto-switch palettes.
    pub theme_runtime: ThemeRuntimeConfig,
    /// Last known foreground host terminal appearance.
    pub host_terminal_appearance: Option<HostAppearance>,
    /// True when the foreground host explicitly reported appearance via Mode 2031.
    pub host_terminal_appearance_explicit: bool,
    /// Settings panel state.
    /// Cached integration recommendations for onboarding/settings UI.
    pub integration_recommendations: Vec<crate::integration::IntegrationRecommendation>,
    /// Cached detection manifest source/version summaries for runtime/API status.
    pub agent_manifest_summaries: Vec<crate::detect::manifest::AgentManifestSummary>,
    /// Cached remote detection manifest update diagnostics for runtime/API status.
    pub agent_manifest_update_status: crate::detect::manifest_update::ManifestUpdateStatus,
    /// Result messages from the latest integration install action.
    pub integration_install_messages: Vec<String>,
    /// Installed or linked plugins known to this running Herdr instance.
    pub(crate) installed_plugins: InstalledPluginRegistry,
    /// Pane ids opened through the plugin pane API.
    pub(crate) plugin_panes: std::collections::HashMap<PaneId, PluginPaneRecord>,
    /// Session-modal terminal popup. This is intentionally outside workspace layouts.
    pub(crate) popup_pane: Option<PopupPaneState>,
    /// Recent plugin action/event command executions.
    pub(crate) plugin_command_logs: Vec<crate::api::schema::PluginCommandLogInfo>,
    pub(crate) next_plugin_command_log_id: u64,
    pub(crate) plugin_commands_in_flight: usize,
    /// Highlight state for the bottom-right global launcher menu.
    /// Resolved host terminal default colors for theming embedded panes.
    pub host_terminal_theme: TerminalTheme,
    /// Last known foreground host terminal cell size in pixels.
    pub(crate) host_cell_size: crate::kitty_graphics::HostCellSize,
    /// Exact pixel provenance only while one confirmed SGR report is dispatched.
    pub(crate) host_mouse_pixels: Option<crate::input::mouse::HostPixels>,
    /// Set when a persisted session snapshot would change.
    pub session_dirty: bool,
    /// Set alongside `session_dirty` by mutations that must reach disk
    /// without the debounce: pane todos are small, rare, user-authored, and
    /// the only session content that exists nowhere else, so a forced
    /// shutdown inside the debounce window must not lose them.
    pub session_save_urgent: bool,
    /// Terminal runtimes that should be shut down by the app/runtime layer
    /// after state has detached their terminal metadata.
    pub(crate) terminal_runtime_shutdowns: Vec<crate::terminal::TerminalId>,
}

impl AppState {
    pub(crate) fn mark_session_dirty(&mut self) {
        self.session_dirty = true;
    }

    /// [`Self::mark_session_dirty`] for content that skips the save debounce.
    pub(crate) fn mark_session_dirty_urgent(&mut self) {
        self.session_dirty = true;
        self.session_save_urgent = true;
    }

    pub(crate) fn remove_alias_shadowed_by_new_pane(&mut self, pane_id: PaneId) {
        self.pane_id_aliases.remove(&pane_id.raw());
    }

    pub fn sound_enabled(&self) -> bool {
        self.sound.enabled
    }

    pub fn toast_delivery(&self) -> ToastDelivery {
        self.toast_config.delivery
    }

    /// Post a herdr toast: shows it as the transient toast (behavior
    /// unchanged) and appends it to the notification log. All production
    /// toast sites go through here so the log stays complete.
    pub(crate) fn post_notification(&mut self, toast: ToastNotification) {
        self.notification_log.post(&toast, now_unix());
        self.toast = Some(toast);
    }

    /// Open the notification center dropdown and mark all entries seen
    /// (same marker path as the `notification.mark_seen` API).
    /// Open the panel. Deliberately leaves read state alone: entries only
    /// quiet down when activated (or via mark-all-read), so the unread badge
    /// tracks what the user actually visited.
    /// Replace the shared name field's contents, cursor at the end.
    pub(crate) fn set_name_input(&mut self, text: impl AsRef<str>) {
        self.name_input =
            crate::ui::text_field::TextField::from_text(text.as_ref(), NAME_INPUT_MAX_CHARS);
    }

    pub(crate) fn open_notification_center(&mut self) {
        self.open_overlay(crate::app::state::Overlay::NotificationCenter(
            NotificationCenterState {
                list: ListCursor::default(),
                hovered_button: None,
            },
        ));
    }

    pub(crate) fn close_notification_center(&mut self) {
        self.close_overlay(crate::app::state::OverlayKind::NotificationCenter);
    }

    /// Empty the notification log (the panel's "Clear all" action). Mutates the
    /// same server-owned log the `notification.clear` API clears, so the panel
    /// and any external consumer stay in agreement.
    pub(crate) fn clear_notifications(&mut self) {
        self.notification_log.clear();
        if let Some(center) = self.notification_center_mut() {
            center.list = ListCursor::default();
            center.hovered_button = None;
        }
    }

    /// Mark every notification read without emptying the log (the panel's
    /// `r` action): the badge quiets, the history stays.
    pub(crate) fn mark_all_notifications_read(&mut self) {
        self.notification_log.mark_all_seen();
    }

    pub(crate) fn notification_center_move_selection(&mut self, delta: isize) {
        let len = self.notification_log.len();
        let visible = self.notification_center_visible_rows();
        let Some(center) = self.notification_center_mut() else {
            return;
        };
        center.list.move_by(delta, len);
        center.list.reveal(visible, len);
    }

    pub(crate) fn notification_center_selected_entry(&self) -> Option<&NotificationEntry> {
        let center = self.notification_center()?;
        self.notification_log
            .entries_newest_first()
            .nth(center.list.selected)
    }

    /// Open the todo panel for a pane. The selection starts at the top of the
    /// presentation order, which is the most urgent outstanding todo.
    pub(crate) fn open_pane_todos(&mut self, pane_id: PaneId) {
        self.open_overlay(crate::app::state::Overlay::PaneTodos(PaneTodoPanelState {
            pane_id,
            list: ListCursor::default(),
            hovered_button: None,
        }));
    }

    /// Closes the panel only. Every caller pairs this with `leave_modal` or an
    /// explicit mode, exactly like `close_notification_center`.
    /// Drop the TUI todo surfaces that pointed at a pane which is going away,
    /// so no panel, modal, or pending confirmation outlives its pane.
    pub(crate) fn forget_pane_todo_ui(&mut self, pane_id: PaneId) {
        if self
            .pane_todos()
            .is_some_and(|panel| panel.pane_id == pane_id)
        {
            self.close_overlay(crate::app::state::OverlayKind::PaneTodos);
            if self.mode == Mode::PaneTodos {
                self.mode = Mode::Terminal;
            }
        }
        if self
            .editing_pane_todo()
            .is_some_and(|edit| edit.pane_id == pane_id)
        {
            self.close_overlay(crate::app::state::OverlayKind::PaneTodoEdit);
            if let Some(navigator) = self.navigator_mut() {
                navigator.suspended_pane_todo_edit = None;
                navigator.purpose = NavigatorPurpose::Goto;
            }
            if self.mode == Mode::PaneTodoEdit {
                self.mode = Mode::Terminal;
            }
        }
        // A panel suspended behind an edit modal points at the pane too. A
        // suspended *board* does not: it is the session's, and it rebuilds
        // from the live store when it comes back.
        if self.pane_todo_edit().is_some_and(|edit| {
            edit.suspended
                .as_ref()
                .and_then(SuspendedTodoSurface::panel)
                .is_some_and(|panel| panel.pane_id == pane_id)
        }) {
            if let Some(edit) = self.pane_todo_edit_mut() {
                edit.suspended = None;
            }
        }
        if self.confirm_close_pane == Some(pane_id) {
            self.confirm_close_pane = None;
        }
        if self.confirm_respawn_pane == Some(pane_id) {
            self.confirm_respawn_pane = None;
        }
    }

    pub(crate) fn close_pane_todos(&mut self) {
        self.close_overlay(crate::app::state::OverlayKind::PaneTodos);
    }

    /// Open the session todo board. The projection is taken here, once, and
    /// the selection starts on the first todo rather than on the heading above
    /// it.
    /// The session-wide outstanding todo count and the highest priority among
    /// them, for the tab bar's todo indicator. One fold over the terminals
    /// map, no allocation: this runs once per rendered frame, beside pane
    /// borders that already read the same per-terminal todo state.
    pub(crate) fn session_outstanding_todos(
        &self,
    ) -> (usize, Option<crate::terminal::todo::TodoPriority>) {
        self.terminals
            .values()
            .fold((0, None), |(count, highest), terminal| {
                (
                    count + terminal.outstanding_todo_count(),
                    highest.max(terminal.highest_outstanding_todo_priority()),
                )
            })
    }

    pub(crate) fn open_todo_board(
        &mut self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) {
        let items = self.todo_board_items(terminal_runtimes);
        self.open_overlay(crate::app::state::Overlay::TodoBoard(TodoBoardState::new(
            items,
        )));
    }

    pub(crate) fn close_todo_board(&mut self) {
        self.close_overlay(crate::app::state::OverlayKind::TodoBoard);
    }

    /// Close whichever todo surface is open. The edit modal is reachable from
    /// both, so its exits must not name one of them.
    pub(crate) fn close_open_todo_surface(&mut self) {
        self.close_pane_todos();
        self.close_todo_board();
    }

    /// Re-settle whichever todo surface is open after a mutation: the panel
    /// re-clamps its cursor, the board rebuilds its projection.
    pub(crate) fn refresh_open_todo_surface(
        &mut self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) {
        self.pane_todos_move_selection(0);
        self.refresh_todo_board(terminal_runtimes);
    }

    /// The todo panel, whether it is the open overlay or suspended behind the
    /// edit modal that was opened over it.
    pub(crate) fn open_pane_todo_panel(&self) -> Option<&PaneTodoPanelState> {
        self.pane_todos().or_else(|| {
            self.editing_pane_todo()
                .and_then(|edit| edit.suspended.as_ref())
                .and_then(SuspendedTodoSurface::panel)
        })
    }

    /// A pane's todos in presentation order. Empty when the pane or its
    /// terminal is gone, which is what lets the panel self-heal instead of
    /// holding a stale target.
    pub(crate) fn pane_todos_in_display_order(
        &self,
        pane_id: PaneId,
    ) -> Vec<&crate::terminal::todo::PaneTodo> {
        self.pane_terminal(pane_id)
            .map(|terminal| terminal.todos_in_display_order())
            .unwrap_or_default()
    }

    /// Move (or, with `0`, re-clamp) the panel selection.
    pub(crate) fn pane_todos_move_selection(&mut self, delta: isize) {
        let Some(pane_id) = self.pane_todos().map(|panel| panel.pane_id) else {
            return;
        };
        let len = self.pane_todos_in_display_order(pane_id).len();
        let visible = self.pane_todo_panel_visible_rows();
        let Some(panel) = self.pane_todos_mut() else {
            return;
        };
        panel.list.move_by(delta, len);
        panel.list.reveal(visible, len);
    }

    /// Move (or, with `0`, re-clamp) the board selection, landing on a todo
    /// rather than on a heading — a heading is rendered in the same list but
    /// is never a destination.
    pub(crate) fn move_todo_board_selection_by(&mut self, delta: isize) {
        let visible = self.todo_board_visible_rows();
        let Some(board) = self.todo_board_mut() else {
            return;
        };
        let len = board.items.len();
        let target = board
            .list
            .selected
            .saturating_add_signed(delta)
            .min(len.saturating_sub(1));
        if let Some(idx) = board.nearest_todo(target, delta >= 0) {
            board.list.select(idx);
        }
        let heading = board.group_heading_index(board.list.selected);
        board.list.reveal_with_context(heading, visible, len);
    }

    /// Move the board's visible window without moving the selection — the
    /// wheel on a list long enough to scan.
    pub(crate) fn scroll_todo_board_by(&mut self, delta: isize) {
        let visible = self.todo_board_visible_rows();
        let Some(board) = self.todo_board_mut() else {
            return;
        };
        let len = board.items.len();
        board.list.scroll_by(delta, visible, len);
    }

    /// The selected todo, cloned so callers can mutate through the API without
    /// holding a borrow of the store.
    pub(crate) fn selected_pane_todo(&self) -> Option<crate::terminal::todo::PaneTodo> {
        let panel = self.open_pane_todo_panel()?;
        self.pane_todos_in_display_order(panel.pane_id)
            .get(panel.list.selected)
            .map(|todo| (*todo).clone())
    }

    /// One todo by id, cloned so callers can mutate through the API without
    /// holding a borrow of the store. The board addresses todos this way
    /// rather than by list position, so its projection cannot point at the
    /// wrong todo after the store moves underneath it.
    pub(crate) fn pane_todo_by_id(
        &self,
        pane_id: PaneId,
        todo_id: u64,
    ) -> Option<crate::terminal::todo::PaneTodo> {
        self.pane_todo_ref(pane_id, todo_id).cloned()
    }

    /// The borrowing form, for the board's render loop: resolving a row must
    /// not build and sort the owning pane's whole presentation order once per
    /// drawn row.
    pub(crate) fn pane_todo_ref(
        &self,
        pane_id: PaneId,
        todo_id: u64,
    ) -> Option<&crate::terminal::todo::PaneTodo> {
        self.pane_terminal(pane_id)?
            .todos()
            .iter()
            .find(|todo| todo.id == todo_id)
    }

    /// The todo the board's selection is on.
    pub(crate) fn selected_todo_board_todo(&self) -> Option<crate::terminal::todo::PaneTodo> {
        let (pane_id, todo_id) = self.todo_board()?.selected_todo()?;
        self.pane_todo_by_id(pane_id, todo_id)
    }

    /// Resolve a todo's link. `None` is a dead link: either the stored target
    /// was already unresolvable at restore, or the pane has since closed.
    /// One definition, so the dimmed chip and the inert click agree.
    pub(crate) fn pane_todo_link_target(
        &self,
        todo: &crate::terminal::todo::PaneTodo,
    ) -> Option<(usize, PaneId)> {
        let pane_id = todo.link.as_ref()?.pane?;
        let ws_idx = self
            .workspaces
            .iter()
            .position(|workspace| workspace.pane_state(pane_id).is_some())?;
        Some((ws_idx, pane_id))
    }

    /// A pane's public identifier, located in the pane's own workspace rather
    /// than the active one — `PaneId` is unique across the session while public
    /// identifiers are workspace-scoped.
    pub(crate) fn session_public_pane_id(&self, pane_id: PaneId) -> Option<String> {
        let ws = self
            .workspaces
            .iter()
            .find(|workspace| workspace.pane_state(pane_id).is_some())?;
        let pane_number = ws.public_pane_number(pane_id)?;
        Some(crate::workspace::public_pane_id_for_number(
            &ws.id,
            pane_number,
        ))
    }

    /// The public identifier a todo's link resolves to right now, or `None`
    /// for a dead link. Derived at presentation time and never stored: public
    /// pane identifiers are positional, so a stored one can go quietly stale
    /// where a derived one is simply absent.
    pub(crate) fn pane_todo_link_public_id(
        &self,
        todo: &crate::terminal::todo::PaneTodo,
    ) -> Option<String> {
        let (_, pane_id) = self.pane_todo_link_target(todo)?;
        self.session_public_pane_id(pane_id)
    }

    /// Open the edit modal on an existing todo, prefilled from the store.
    pub(crate) fn open_pane_todo_edit(&mut self, pane_id: PaneId, todo_id: u64) {
        let Some(todo) = self
            .pane_terminal(pane_id)
            .and_then(|terminal| terminal.todos().iter().find(|todo| todo.id == todo_id))
            .cloned()
        else {
            return;
        };
        // The surface the modal opened over is suspended onto it, so closing
        // the modal hands it back rather than that surface outliving its own
        // mode.
        let suspended = self.take_suspended_todo_surface();
        self.open_overlay(crate::app::state::Overlay::PaneTodoEdit(
            PaneTodoEditState {
                pane_id,
                todo_id: Some(todo.id),
                text: crate::ui::text_field::TextField::from_text(
                    &todo.text,
                    crate::terminal::todo::MAX_TODO_TEXT_LEN,
                ),
                priority: todo.priority,
                link: PaneTodoEditLink::Keep,
                done: todo.done,
                suspended,
            },
        ));
    }

    /// Open the edit modal on a brand-new todo for a pane.
    pub(crate) fn open_new_pane_todo(&mut self, pane_id: PaneId) {
        let suspended = self.take_suspended_todo_surface();
        self.open_overlay(crate::app::state::Overlay::PaneTodoEdit(
            PaneTodoEditState {
                pane_id,
                todo_id: None,
                text: crate::ui::text_field::TextField::new(
                    crate::terminal::todo::MAX_TODO_TEXT_LEN,
                ),
                priority: crate::terminal::todo::TodoPriority::default(),
                link: PaneTodoEditLink::Keep,
                done: false,
                suspended,
            },
        ));
    }

    /// Close the edit modal, reopening the surface it was suspended over.
    /// Answers whether one came back, since the caller's way out depends on it.
    pub(crate) fn close_pane_todo_edit(&mut self) -> bool {
        let Some(edit) = self.take_pane_todo_edit() else {
            return false;
        };
        let Some(surface) = edit.suspended else {
            return false;
        };
        self.open_overlay(surface.into_overlay());
        true
    }

    /// Take whichever todo surface is open, to be carried by the edit modal
    /// about to replace it.
    fn take_suspended_todo_surface(&mut self) -> Option<SuspendedTodoSurface> {
        self.take_pane_todos()
            .map(SuspendedTodoSurface::Panel)
            .or_else(|| self.take_todo_board().map(SuspendedTodoSurface::Board))
    }

    /// How a staged link target is named in the edit modal. Deliberately the
    /// same chain the server captures on save, so the preview and the label
    /// that ends up stored agree.
    pub(crate) fn pane_link_target_label(&self, target: PaneId) -> String {
        let Some(terminal) = self.pane_terminal(target) else {
            return "pane".to_string();
        };
        terminal
            .manual_label
            .clone()
            .or_else(|| terminal.effective_agent_label().map(str::to_string))
            .or_else(|| crate::app::actions::launch_label(terminal.launch_argv.as_ref()))
            .unwrap_or_else(|| "pane".to_string())
    }

    /// Toggle the done state of the todo being edited. Inert while composing a
    /// new todo: `todo.add` carries no `done`, so there would be nothing to
    /// save it to.
    pub(crate) fn toggle_pane_todo_edit_done(&mut self) {
        let Some(edit) = self.pane_todo_edit_mut() else {
            return;
        };
        if edit.todo_id.is_none() {
            return;
        }
        edit.done = !edit.done;
    }

    pub(crate) fn cycle_pane_todo_edit_priority(&mut self) {
        let Some(edit) = self.pane_todo_edit_mut() else {
            return;
        };
        edit.priority = match edit.priority {
            crate::terminal::todo::TodoPriority::Low => crate::terminal::todo::TodoPriority::Normal,
            crate::terminal::todo::TodoPriority::Normal => {
                crate::terminal::todo::TodoPriority::High
            }
            crate::terminal::todo::TodoPriority::High => crate::terminal::todo::TodoPriority::Low,
        };
    }

    /// Where the modal's link control currently points, resolved live against
    /// the staged choice rather than the stored one. `None` when there is
    /// nothing to follow — no link, an explicit clear, or a target that has
    /// gone — which is also exactly when the row offers no `go`.
    pub(crate) fn pane_todo_edit_link_target(&self) -> Option<(usize, PaneId)> {
        let edit = self.pane_todo_edit()?;
        let target = match edit.link {
            PaneTodoEditLink::Clear => return None,
            PaneTodoEditLink::Set(target) => target,
            PaneTodoEditLink::Keep => {
                let todo_id = edit.todo_id?;
                let terminal = self.pane_terminal(edit.pane_id)?;
                let todo = terminal.todos().iter().find(|todo| todo.id == todo_id)?;
                todo.link.as_ref()?.pane?
            }
        };
        let ws_idx = self
            .workspaces
            .iter()
            .position(|workspace| workspace.pane_state(target).is_some())?;
        Some((ws_idx, target))
    }

    /// What the modal's link row shows for the current choice: the target's
    /// public identifier first and its label after it, the same composition
    /// the panel's chip uses. A dead link resolves to no identifier and shows
    /// its label alone.
    pub(crate) fn pane_todo_edit_link_label(&self) -> String {
        let Some(edit) = self.pane_todo_edit() else {
            return String::new();
        };
        let (public_id, label) = match edit.link {
            PaneTodoEditLink::Clear => return "none".to_string(),
            PaneTodoEditLink::Keep => {
                let Some(todo) = edit.todo_id.and_then(|todo_id| {
                    let terminal = self.pane_terminal(edit.pane_id)?;
                    terminal.todos().iter().find(|todo| todo.id == todo_id)
                }) else {
                    return "none".to_string();
                };
                let Some(link) = todo.link.as_ref() else {
                    return "none".to_string();
                };
                (self.pane_todo_link_public_id(todo), link.label.clone())
            }
            PaneTodoEditLink::Set(target) => (
                self.session_public_pane_id(target),
                self.pane_link_target_label(target),
            ),
        };
        match public_id {
            Some(id) if label.is_empty() => id,
            Some(id) => format!("{id} · {label}"),
            None => label,
        }
    }

    pub fn agent_border_labels_enabled(&self) -> bool {
        self.show_agent_labels_on_pane_borders
    }

    /// Border line color for a pane; falls back to the theme accent (focused)
    /// or the theme's muted border color (unfocused) when unset.
    pub fn pane_border_color(&self, focused: bool) -> Color {
        // While `prefix+i`'s labels are up the whole mode is red, like the
        // close-workspace confirmation, over any configured border colour.
        if self.display_panes().is_some() {
            return if focused {
                self.palette.red
            } else {
                self.palette.muted_red()
            };
        }
        if focused {
            self.pane_border_active_color.unwrap_or(self.palette.accent)
        } else {
            self.pane_border_inactive_color
                .unwrap_or(self.palette.overlay0)
        }
    }

    /// Border title color for a pane; falls back to the matching border color
    /// (which itself falls back to the theme) when unset.
    pub fn pane_title_color(&self, focused: bool) -> Color {
        let explicit = if focused {
            self.pane_title_active_color
        } else {
            self.pane_title_inactive_color
        };
        explicit.unwrap_or_else(|| self.pane_border_color(focused))
    }

    /// The terminal backing a pane, wherever that pane lives. Todos are stored
    /// on `TerminalState`, so every todo surface resolves through here.
    pub(crate) fn pane_terminal(&self, pane_id: PaneId) -> Option<&crate::terminal::TerminalState> {
        let pane = self
            .workspaces
            .iter()
            .find_map(|workspace| workspace.pane_state(pane_id))?;
        self.terminals.get(&pane.attached_terminal_id)
    }

    /// Colour for a todo's priority: the pane indicator's outstanding state,
    /// and the priority chips in the panel and edit modal. `ui.pane_todo_color`
    /// pins it; `None` has no priority to show and reads muted. The
    /// indicator's done and empty states are tones of their own — see
    /// `PaneTodoIndicatorState::color`.
    pub fn pane_todo_indicator_color(
        &self,
        priority: Option<crate::terminal::todo::TodoPriority>,
    ) -> Color {
        let Some(priority) = priority else {
            return self.palette.overlay0;
        };
        if let Some(color) = self.pane_todo_color {
            return color;
        }
        match priority {
            crate::terminal::todo::TodoPriority::High => self.palette.red,
            crate::terminal::todo::TodoPriority::Normal => self.palette.yellow,
            crate::terminal::todo::TodoPriority::Low => self.palette.blue,
        }
    }

    /// Background of the active space/agent band in the sidebar; falls back
    /// to the theme's subtle highlight when unset.
    /// Per-state colors for sidebar state glyphs and state text:
    /// `[ui.state_colors]` overrides resolved against the theme palette.
    pub fn state_icon_colors(&self) -> StateIconColors {
        let overrides = self.state_color_overrides;
        StateIconColors {
            working: overrides.working.unwrap_or(self.palette.yellow),
            idle: overrides.idle.unwrap_or(self.palette.green),
            done: overrides.done.unwrap_or(self.palette.teal),
            blocked: overrides.blocked.unwrap_or(self.palette.red),
            unknown: overrides.unknown.unwrap_or(self.palette.overlay0),
            background: overrides
                .background
                .or(overrides.working)
                .unwrap_or(self.palette.yellow),
        }
    }

    /// Per-state glyphs for sidebar state icons: the active
    /// `ui.status_indicators` set with `[ui.state_symbols]` overrides on top.
    pub fn state_icon_symbols(&self) -> StateIconSymbols<'_> {
        let defaults = StateIconSymbols::for_style(self.status_indicators);
        let overrides = &self.state_symbol_overrides;
        StateIconSymbols {
            working: overrides.working.as_deref().unwrap_or(defaults.working),
            idle: overrides.idle.as_deref().unwrap_or(defaults.idle),
            done: overrides.done.as_deref().unwrap_or(defaults.done),
            blocked: overrides.blocked.as_deref().unwrap_or(defaults.blocked),
            unknown: overrides.unknown.as_deref().unwrap_or(defaults.unknown),
            background: overrides
                .background
                .as_deref()
                .unwrap_or(defaults.background),
            background_alt: overrides
                .background_alt
                .as_deref()
                .unwrap_or(defaults.background_alt),
        }
    }

    /// Whether the working spinner should be ticking: it is on, the sidebar
    /// is on screen, and at least one agent is working. Cheap — one pass over
    /// the terminal table, which is tens of entries at most.
    pub fn spinner_active(&self) -> bool {
        self.status_spinner == crate::config::StatusSpinnerConfig::On
            && self
                .terminals
                .values()
                .any(|terminal| terminal.state == crate::detect::AgentState::Working)
    }

    /// The frame agent rows draw for a working agent, or `None` when the
    /// spinner is off and the static glyph should show.
    pub fn working_spinner_frame(&self) -> Option<u8> {
        (self.status_spinner == crate::config::StatusSpinnerConfig::On)
            .then_some(self.spinner_frame)
    }

    pub(crate) fn pane_exposes_host_cursor(
        &self,
        _ws_idx: usize,
        _pane_id: crate::layout::PaneId,
    ) -> bool {
        true
    }

    pub(crate) fn integration_updates_available(&self) -> bool {
        self.integration_recommendations
            .iter()
            .any(|item| item.state == crate::integration::IntegrationStatusKind::Outdated)
    }

    pub(crate) fn refresh_agent_manifest_summaries(&mut self) {
        self.agent_manifest_summaries = crate::detect::manifest::manifest_summaries();
    }

    pub(crate) fn global_menu_attention_badge_visible(&self) -> bool {
        self.update_available.is_some() || self.integration_updates_available()
    }

    pub(crate) fn global_menu_item_has_badge(&self, item: &str) -> bool {
        (item == "update ready" && self.update_available.is_some())
            || (item == "settings" && self.integration_updates_available())
    }

    pub(crate) fn settings_section_has_badge(&self, section: SettingsSection) -> bool {
        section == SettingsSection::Integrations && self.integration_updates_available()
    }

    pub(crate) fn app_surface_pane_ids(&self) -> std::collections::HashSet<PaneId> {
        let mut pane_ids = std::collections::HashSet::new();
        if let Some(popup) = &self.popup_pane {
            pane_ids.insert(popup.pane_id);
        }
        let Some(tab) = self
            .active
            .and_then(|ws_idx| self.workspaces.get(ws_idx))
            .and_then(crate::workspace::Workspace::active_tab)
        else {
            return pane_ids;
        };
        if tab.zoomed {
            pane_ids.insert(tab.layout.focused());
        } else {
            pane_ids.extend(tab.panes.keys().copied());
        }
        pane_ids
    }

    pub(crate) fn focused_pane_requests_mouse_capture_from(
        &self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) -> bool {
        self.mode == Mode::Terminal
            && self
                .active
                .and_then(|idx| self.focused_runtime_in_workspace(terminal_runtimes, idx))
                .is_some_and(crate::terminal::TerminalRuntime::mouse_reporting_enabled)
    }

    pub(crate) fn should_capture_host_mouse_from(
        &self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ) -> bool {
        self.mouse_capture
            || self.popup_pane.is_some()
            || self.focused_pane_requests_mouse_capture_from(terminal_runtimes)
    }

    pub fn is_prefix_key(&self, key: &crate::input::TerminalKey) -> bool {
        crate::config::terminal_key_matches_combo(key, (self.prefix_code, self.prefix_mods))
    }

    /// The size, (cols, rows), panes are laid out at while no client is
    /// attached: a size a live handoff carried, then the remembered client
    /// size, then the configured headless size.
    pub(crate) fn no_client_size(&self, handoff_client_size: Option<(u16, u16)>) -> (u16, u16) {
        handoff_client_size
            .or(self.last_client_size.filter(|_| self.remember_client_size))
            .unwrap_or(self.headless_size)
    }

    /// Remembers the foreground client's size, (cols, rows), unless
    /// remembering is off or the size is below the floor. A change is saved
    /// with the session.
    pub(crate) fn remember_foreground_client_size(&mut self, size: (u16, u16)) {
        if !self.remember_client_size {
            return;
        }
        let Some(size) = rememberable_client_size(size) else {
            return;
        };
        if self.last_client_size != Some(size) {
            self.last_client_size = Some(size);
            self.mark_session_dirty();
        }
    }

    pub fn estimate_pane_size(&self) -> (u16, u16) {
        // With no client attached the view is not what anyone sees, and its
        // first pane can be any size; new panes get the no-client size.
        if let Some((cols, rows)) = self.detached_pane_size {
            return (rows, cols);
        }
        if let Some(info) = self.view.pane_infos.first() {
            (info.rect.height, info.rect.width)
        } else {
            (self.headless_size.1, self.headless_size.0)
        }
    }

    /// Returns true when the given (workspace, tab, pane) refers to the
    /// currently focused pane in the active workspace's active tab.
    pub(crate) fn runtime_for_pane_in_workspace<'a>(
        &'a self,
        terminal_runtimes: &'a crate::terminal::TerminalRuntimeRegistry,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<&'a crate::terminal::TerminalRuntime> {
        #[cfg(test)]
        if let Some(runtime) = self.workspaces.get(ws_idx)?.test_runtimes.get(&pane_id) {
            return Some(runtime);
        }
        #[cfg(test)]
        if let Some(runtime) = self
            .workspaces
            .get(ws_idx)?
            .tabs
            .iter()
            .find_map(|tab| tab.runtimes.get(&pane_id))
        {
            return Some(runtime);
        }
        let terminal_id = self.workspaces.get(ws_idx)?.terminal_id(pane_id)?;
        terminal_runtimes.get(terminal_id)
    }

    #[cfg(test)]
    pub(crate) fn runtime_for_pane<'a>(
        &'a self,
        terminal_runtimes: &'a crate::terminal::TerminalRuntimeRegistry,
        pane_id: crate::layout::PaneId,
    ) -> Option<&'a crate::terminal::TerminalRuntime> {
        self.workspaces.iter().find_map(|ws| {
            #[cfg(test)]
            if let Some(runtime) = ws.test_runtimes.get(&pane_id) {
                return Some(runtime);
            }
            #[cfg(test)]
            if let Some(runtime) = ws.tabs.iter().find_map(|tab| tab.runtimes.get(&pane_id)) {
                return Some(runtime);
            }
            let terminal_id = ws.terminal_id(pane_id)?;
            terminal_runtimes.get(terminal_id)
        })
    }

    pub(crate) fn focused_runtime_in_workspace<'a>(
        &'a self,
        terminal_runtimes: &'a crate::terminal::TerminalRuntimeRegistry,
        ws_idx: usize,
    ) -> Option<&'a crate::terminal::TerminalRuntime> {
        let ws = self.workspaces.get(ws_idx)?;
        let pane_id = ws.focused_pane_id()?;
        self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)
    }

    pub fn is_active_pane(
        &self,
        ws_idx: usize,
        tab_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> bool {
        let Some(active_ws_idx) = self.active else {
            return false;
        };
        if ws_idx != active_ws_idx {
            return false;
        }
        let Some(ws) = self.workspaces.get(ws_idx) else {
            return false;
        };
        if tab_idx != ws.active_tab_index() {
            return false;
        }
        ws.active_tab().map(|tab| tab.layout.focused()) == Some(pane_id)
    }
}

#[cfg(test)]
pub fn key_matches(
    key: &crossterm::event::KeyEvent,
    expected_code: KeyCode,
    expected_mods: KeyModifiers,
) -> bool {
    crate::config::terminal_key_matches_combo(
        &crate::input::TerminalKey::from(*key),
        (expected_code, expected_mods),
    )
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
impl AppState {
    /// Create an AppState for testing — no channels, no PTYs.
    pub fn test_new() -> Self {
        Self {
            terminals: std::collections::HashMap::new(),
            direct_attach_resize_locks: std::collections::HashSet::new(),
            pane_id_aliases: std::collections::HashMap::new(),
            public_pane_id_aliases: std::collections::HashMap::new(),
            workspaces: Vec::new(),
            active: None,
            previous_pane_focus: None,
            selected: 0,
            mode: Mode::Navigate,
            confirm_close_workspace_id: None,
            should_quit: false,
            detach_exits: false,
            detach_requested: false,
            request_new_workspace: false,
            request_new_tab: false,
            request_new_linked_worktree: None,
            request_open_existing_worktree: None,
            request_new_workspace_cwd: None,
            request_remove_linked_worktree: None,
            request_submit_worktree_create: false,
            request_submit_worktree_open: false,
            request_submit_worktree_remove: false,
            request_reload_config: false,
            request_client_config_reload: false,
            request_clipboard_write: None,
            pending_app_scroll_sends: Vec::new(),
            creating_new_tab: false,
            requested_new_tab_name: None,
            pending_workspace_create_cwd: None,
            rename_pane_target: None,
            confirm_close_pane: None,
            confirm_respawn_pane: None,
            overlay: None,
            worktree_directory: std::path::PathBuf::from("/tmp/herdr-worktrees"),
            collapsed_space_keys: std::collections::HashSet::new(),
            request_complete_onboarding: false,
            name_input: crate::ui::text_field::TextField::new(
                crate::app::state::NAME_INPUT_MAX_CHARS,
            ),
            name_input_replace_on_type: false,
            copy_mode: None,
            app_scroll: None,
            workspace_scroll: 0,
            agent_panel_scroll: 0,
            sidebar_followed_workspace: None,
            sidebar_followed_agent: None,
            workspace_list_follow_active: true,
            agent_panel_follow_active: true,
            tab_scroll: 0,
            tab_scroll_follow_active: true,
            mobile_switcher_scroll: 0,
            view: ViewState {
                layout: ViewLayout::Desktop,
                sidebar_rect: Rect::default(),
                workspace_card_areas: Vec::new(),
                tab_bar_rect: Rect::default(),
                tab_hit_areas: Vec::new(),
                tab_scroll_left_hit_area: Rect::default(),
                tab_scroll_right_hit_area: Rect::default(),
                new_tab_hit_area: Rect::default(),
                todo_hit_area: Rect::default(),
                notification_hit_area: Rect::default(),
                terminal_area: Rect::default(),
                mobile_header_rect: Rect::default(),
                mobile_menu_hit_area: Rect::default(),
                toast_hit_area: Rect::default(),
                pane_infos: Vec::new(),
                split_borders: Vec::new(),
            },
            drag: None,
            resize_labels_until: None,
            resize_labels_window: false,
            workspace_presses: std::collections::HashMap::new(),
            tab_presses: std::collections::HashMap::new(),
            selection: None,
            selection_autoscroll: None,
            update_available: None,
            update_install_command: "herdr update".into(),
            latest_release_notes_available: false,
            update_dismissed: false,
            config_diagnostic: None,
            toast: None,
            notification_log: NotificationLog::default(),
            pending_agent_notifications: std::collections::HashMap::new(),
            copy_feedback: None,
            outer_terminal_focus: None,
            prefix_code: KeyCode::Char('b'),
            prefix_mods: KeyModifiers::CONTROL,
            headless_size: (
                crate::config::DEFAULT_HEADLESS_COLS,
                crate::config::DEFAULT_HEADLESS_ROWS,
            ),
            detached_pane_size: None,
            last_client_size: None,
            remember_client_size: true,
            default_sidebar_width: 26,
            sidebar_width: 26,
            sidebar_min_width: 18,
            sidebar_max_width: 36,
            mobile_width_threshold: crate::config::DEFAULT_MOBILE_WIDTH_THRESHOLD,
            sidebar_width_source: SidebarWidthSource::ConfigDefault,
            sidebar_width_auto: false,
            sidebar_collapsed: false,
            sidebar_collapsed_mode: crate::config::SidebarCollapsedModeConfig::Compact,
            layout_cycle_index: 0,
            sidebar_section_split: 0.5,
            agent_panel_sort: AgentPanelSort::Spaces,
            status_indicators: crate::config::StatusIndicatorStyle::Dots,
            status_spinner: crate::config::StatusSpinnerConfig::On,
            status_spinner_interval: std::time::Duration::from_millis(
                crate::config::DEFAULT_STATUS_SPINNER_MS,
            ),
            display_panes_duration: std::time::Duration::from_millis(
                crate::config::DEFAULT_DISPLAY_PANES_MS,
            ),
            spinner_frame: 0,
            agent_view_override: None,
            sidebar_agents: crate::config::AgentsSidebarConfig::default(),
            sidebar_spaces: crate::config::SpacesSidebarConfig::default(),
            workspace_sort: WorkspaceSort::Manual,
            sort_motion_bubble: true,
            sort_motion_timing: crate::ui::list_motion::ListMotionTiming {
                settle: std::time::Duration::from_millis(2000),
                step: std::time::Duration::from_millis(150),
                easing: crate::ui::list_motion::ListMotionEasing::Linear,
            },
            workspace_list_motion: crate::ui::list_motion::ListMotion::new(),
            agent_panel_motion: crate::ui::list_motion::ListMotion::new(),
            sidebar_style: crate::config::SidebarStyleConfig::Default,
            state_color_overrides: StateColorOverrides::default(),
            state_symbol_overrides: StateSymbolOverrides::default(),
            notification_center_position: crate::config::NotificationCenterPositionConfig::TopRight,
            next_agent_state_change_seq: 0,
            mouse_capture: true,
            copy_on_select: true,
            right_click_passthrough_modifiers: None,
            right_click_passthrough: None,
            redraw_on_focus_gained: true,
            mouse_scroll_lines: crate::config::DEFAULT_MOUSE_SCROLL_LINES,
            confirm_close: true,
            prompt_new_tab_name: true,
            prompt_new_workspace_name: false,
            pane_borders: true,
            pane_outer_borders: true,
            pane_scrollbars: true,
            pane_gaps: false,
            show_agent_labels_on_pane_borders: false,
            show_pane_todo_indicator: true,
            hide_tab_bar_when_single_tab: false,
            tab_bar_position: TabBarPositionConfig::Top,
            tab_bar_right: Vec::new(),
            tab_bar_right_separator: " ".into(),
            show_workspace_numbers: false,
            show_agent_numbers: false,
            show_host: false,
            host_label: None,
            pane_history_persistence: false,
            reveal_hidden_cursor_for_cjk_ime: false,
            cjk_ime_agent_filter_configured: false,
            cjk_ime_agents: Vec::new(),
            cjk_ime_cursor_shape: 2, // steady_block
            switch_ascii_input_source_in_prefix: false,
            kitty_graphics_enabled: false,
            default_shell: String::new(),
            shell_mode: crate::config::ShellModeConfig::Auto,
            new_terminal_cwd: NewTerminalCwdConfig::Follow,
            pane_scrollback_limit_bytes: crate::config::DEFAULT_SCROLLBACK_LIMIT_BYTES,
            accent: Color::Cyan,
            workspace_number_color: None,
            agent_number_color: None,
            workspace_number_prefix: String::new(),
            agent_number_prefix: String::new(),
            pane_border_active_color: None,
            pane_border_inactive_color: None,
            pane_border_active_style: crate::config::PaneBorderActiveStyleConfig::Light,
            pane_title_active_color: None,
            pane_title_inactive_color: None,
            pane_todo_color: None,
            sidebar_active_border: crate::config::SidebarActiveBorderConfig::Off,
            sidebar_overflow: crate::config::SidebarOverflowConfig::default(),
            sidebar_fog: crate::config::Config::default().sidebar_fog().0,
            sidebar_fog_tint: crate::config::Config::default().sidebar_fog().1,
            sidebar_fog_style: crate::config::SidebarFogStyle::default(),
            sidebar_fade: crate::config::Config::default().sidebar_fade(),
            pane_active_bg: None,
            pane_inactive_bg: None,
            dim_inactive_panes: false,
            inactive_pane_dim: 0,
            sound: SoundConfig {
                enabled: false,
                ..SoundConfig::default()
            },
            local_sound_playback: false,
            toast_config: ToastConfig::default(),
            keybinds: Keybinds::default(),
            palette: Palette::catppuccin(),
            theme_name: "catppuccin".to_string(),
            theme_runtime: ThemeRuntimeConfig {
                manual_name: "catppuccin".to_string(),
                dark_name: "catppuccin".to_string(),
                light_name: "catppuccin-latte".to_string(),
                auto_switch: false,
                custom: None,
                legacy_accent: None,
            },
            host_terminal_appearance: None,
            host_terminal_appearance_explicit: false,
            integration_recommendations: Vec::new(),
            agent_manifest_summaries: Vec::new(),
            agent_manifest_update_status:
                crate::detect::manifest_update::ManifestUpdateStatus::default(),
            integration_install_messages: Vec::new(),
            installed_plugins: std::collections::HashMap::new(),
            plugin_panes: std::collections::HashMap::new(),
            popup_pane: None,
            plugin_command_logs: Vec::new(),
            next_plugin_command_log_id: 1,
            plugin_commands_in_flight: 0,
            host_terminal_theme: TerminalTheme::default(),
            host_cell_size: crate::kitty_graphics::HostCellSize::default(),
            host_mouse_pixels: None,
            session_dirty: false,
            session_save_urgent: false,
            terminal_runtime_shutdowns: Vec::new(),
        }
    }

    /// Populate missing `TerminalState` entries for every pane so tests that
    /// read or write terminal metadata don't need to manually create them.
    pub fn ensure_test_terminals(&mut self) {
        use crate::terminal::TerminalState;
        for ws in &self.workspaces {
            for tab in &ws.tabs {
                for pane in tab.panes.values() {
                    if !self.terminals.contains_key(&pane.attached_terminal_id) {
                        let cwd = ws.identity_cwd.clone();
                        self.terminals.insert(
                            pane.attached_terminal_id.clone(),
                            TerminalState::new(pane.attached_terminal_id.clone(), cwd),
                        );
                    }
                }
            }
        }
    }

    pub fn test_with_adversarial_identity_state() -> Self {
        let mut state = Self::test_new();
        state.workspaces = vec![crate::workspace::Workspace::test_adversarial_identity_state()];
        state.active = Some(0);
        state.selected = 0;
        state.ensure_test_terminals();
        state
    }

    pub fn assert_invariants_for_test(&self) {
        // Mode and overlay are one fact. An open overlay names the mode, and a
        // mode that names an overlay has that overlay open — the pairing that
        // used to be convention between `Mode` and ten-plus parallel
        // `Option<XState>` fields.
        if let Some(overlay) = self.overlay.as_ref() {
            assert_eq!(
                self.mode,
                overlay.mode(),
                "the open overlay ({:?}) and the active mode disagree",
                overlay.kind()
            );
        } else if let Some(kind) = OverlayKind::ALL
            .iter()
            .find(|kind| kind.mode() == self.mode)
        {
            panic!("mode {:?} names {kind:?} but no overlay is open", self.mode);
        }

        // A pin order names one place in the pinned block: no two spaces share it.
        let mut pin_orders: Vec<u64> = self
            .workspaces
            .iter()
            .filter_map(|ws| ws.pin_order)
            .collect();
        pin_orders.sort_unstable();
        assert!(
            pin_orders.windows(2).all(|pair| pair[0] != pair[1]),
            "two workspaces share a pin order: {pin_orders:?}"
        );

        if self.workspaces.is_empty() {
            assert!(
                self.active.is_none(),
                "empty app state must not have active workspace {:?}",
                self.active
            );
            assert_eq!(
                self.selected, 0,
                "empty app state should keep selected workspace at 0"
            );
            assert!(
                self.pane_id_aliases.is_empty(),
                "empty app state must not keep raw pane aliases"
            );
            assert!(
                self.public_pane_id_aliases.is_empty(),
                "empty app state must not keep public pane aliases"
            );
            assert!(
                self.previous_pane_focus.is_none(),
                "empty app state must not keep previous pane focus"
            );
            assert!(
                self.plugin_panes.is_empty(),
                "empty app state must not keep plugin pane records"
            );
            assert!(
                self.pending_agent_notifications.is_empty(),
                "empty app state must not keep pending agent notifications"
            );
            assert!(
                self.copy_mode.is_none(),
                "empty app state must not keep copy mode"
            );
            assert!(
                self.rename_pane_target.is_none(),
                "empty app state must not keep rename pane target"
            );
            // Deliberately only the empty-state assertion: with panes around,
            // the panel resolves its pane on every read and renders nothing
            // once it is gone, so asserting liveness would encode a stronger
            // contract than the code keeps. `rename_pane_target` is asserted
            // more strongly because a save consumes it.
            assert!(
                self.pane_todos().is_none(),
                "empty app state must not keep a pane todo panel"
            );
            assert!(
                self.pane_todo_edit().is_none(),
                "empty app state must not keep a pane todo edit buffer"
            );
            assert!(
                self.selection.is_none(),
                "empty app state must not keep text selection"
            );
            assert!(
                self.selection_autoscroll.is_none(),
                "empty app state must not keep selection autoscroll"
            );
            if let Some(toast) = &self.toast {
                assert!(
                    toast.target.is_none(),
                    "empty app state must not keep pane-targeted toast"
                );
            }
            assert!(
                self.right_click_passthrough.is_none(),
                "empty app state must not keep right-click passthrough gesture"
            );
            assert!(
                self.drag.is_none(),
                "empty app state must not keep drag state"
            );
            assert!(
                self.workspace_presses.is_empty(),
                "empty app state must not keep workspace press state"
            );
            assert!(
                self.tab_presses.is_empty(),
                "empty app state must not keep tab press state"
            );
            assert!(
                self.context_menu().is_none(),
                "empty app state must not keep context menu"
            );
            assert!(
                self.host_mouse_pixels.is_none(),
                "empty app state must not keep host mouse pixel provenance"
            );
            return;
        }

        assert!(
            self.selected < self.workspaces.len(),
            "selected workspace {} out of bounds for {} workspaces",
            self.selected,
            self.workspaces.len()
        );
        let active = self
            .active
            .expect("non-empty app state must have active workspace");
        assert!(
            active < self.workspaces.len(),
            "active workspace {} out of bounds for {} workspaces",
            active,
            self.workspaces.len()
        );

        let mut workspace_ids = std::collections::HashSet::new();
        let mut workspace_id_to_idx = std::collections::HashMap::new();
        let mut pane_ids = std::collections::HashSet::new();
        let mut attached_terminal_ids = std::collections::HashSet::new();
        for (ws_idx, ws) in self.workspaces.iter().enumerate() {
            assert!(
                workspace_ids.insert(ws.id.clone()),
                "duplicate workspace id {} at workspace index {}",
                ws.id,
                ws_idx
            );
            workspace_id_to_idx.insert(ws.id.clone(), ws_idx);
            ws.assert_invariants_for_test();

            for tab in &ws.tabs {
                for (pane_id, pane) in &tab.panes {
                    assert!(
                        pane_ids.insert(*pane_id),
                        "pane {:?} appears in more than one workspace",
                        pane_id
                    );
                    assert!(
                        attached_terminal_ids.insert(pane.attached_terminal_id.clone()),
                        "terminal {} is attached to more than one app pane",
                        pane.attached_terminal_id
                    );
                    assert!(
                        self.terminals.contains_key(&pane.attached_terminal_id),
                        "pane {:?} is attached to missing terminal {}",
                        pane_id,
                        pane.attached_terminal_id
                    );
                }
            }
        }

        let assert_live_pane = |pane_id: PaneId, context: &str| {
            assert!(
                pane_ids.contains(&pane_id),
                "{context} references missing pane {:?}",
                pane_id
            );
        };
        let assert_workspace_pane = |workspace_id: &str, pane_id: PaneId, context: &str| {
            let ws_idx = workspace_id_to_idx
                .get(workspace_id)
                .copied()
                .unwrap_or_else(|| panic!("{context} references missing workspace {workspace_id}"));
            assert!(
                self.workspaces[ws_idx].pane_state(pane_id).is_some(),
                "{context} references pane {:?} outside workspace {}",
                pane_id,
                workspace_id
            );
        };
        let assert_workspace_index = |ws_idx: usize, context: &str| {
            assert!(
                ws_idx < self.workspaces.len(),
                "{context} references workspace index {} out of bounds for {} workspaces",
                ws_idx,
                self.workspaces.len()
            );
        };
        let assert_tab_index = |ws_idx: usize, tab_idx: usize, context: &str| {
            assert_workspace_index(ws_idx, context);
            assert!(
                tab_idx < self.workspaces[ws_idx].tabs.len(),
                "{context} references tab index {} out of bounds for workspace {} with {} tabs",
                tab_idx,
                ws_idx,
                self.workspaces[ws_idx].tabs.len()
            );
        };

        for (&raw, &pane_id) in &self.pane_id_aliases {
            assert_live_pane(pane_id, &format!("raw pane alias {raw}"));
        }
        for (public_id, &pane_id) in &self.public_pane_id_aliases {
            assert_live_pane(pane_id, &format!("public pane alias {public_id}"));
        }
        if let Some(focus) = &self.previous_pane_focus {
            assert_workspace_pane(&focus.workspace_id, focus.pane_id, "previous pane focus");
        }
        if let Some(toast) = &self.toast {
            if let Some(target) = &toast.target {
                assert_workspace_pane(&target.workspace_id, target.pane_id, "toast target");
            }
        }
        for (&pane_id, notification) in &self.pending_agent_notifications {
            assert_eq!(
                pane_id, notification.pane_id,
                "pending agent notification map key must match payload pane id"
            );
            assert_workspace_pane(
                &notification.workspace_id,
                notification.pane_id,
                "pending agent notification",
            );
        }
        if let Some(popup) = &self.popup_pane {
            assert!(
                self.terminals.contains_key(&popup.terminal_id),
                "popup {:?} references missing terminal {}",
                popup.pane_id,
                popup.terminal_id
            );
            assert!(
                !attached_terminal_ids.contains(&popup.terminal_id),
                "popup terminal {} must not be attached to a tiled pane",
                popup.terminal_id
            );
        }
        for &pane_id in self.plugin_panes.keys() {
            assert_live_pane(pane_id, "plugin pane record");
        }
        if let Some(copy_mode) = &self.copy_mode {
            assert_live_pane(copy_mode.pane_id, "copy mode");
        }
        if let Some(pane_id) = self.rename_pane_target {
            assert_live_pane(pane_id, "rename pane target");
        }
        if let Some(selection) = &self.selection {
            assert_live_pane(selection.pane_id, "text selection");
        } else {
            assert!(
                self.selection_autoscroll.is_none(),
                "selection autoscroll must not remain without an active text selection"
            );
        }
        if let Some(gesture) = &self.right_click_passthrough {
            assert_live_pane(gesture.pane_info.id, "right-click passthrough gesture");
        }
        if let Some(drag) = &self.drag {
            match &drag.target {
                DragTarget::WorkspaceReorder {
                    source_ws_idx,
                    drop_target,
                    ..
                } => {
                    assert_workspace_index(*source_ws_idx, "workspace drag source");
                    if let Some(WorkspaceDropTarget::Before(ws_idx)) = drop_target {
                        assert_workspace_index(*ws_idx, "workspace drag target");
                    }
                }
                DragTarget::TabReorder {
                    ws_idx,
                    source_tab_idx,
                    insert_idx,
                    ..
                } => {
                    assert_tab_index(*ws_idx, *source_tab_idx, "tab drag source");
                    if let Some(insert_idx) = insert_idx {
                        assert!(
                            *insert_idx <= self.workspaces[*ws_idx].tabs.len(),
                            "tab drag insert index {} out of bounds for workspace {} with {} tabs",
                            insert_idx,
                            ws_idx,
                            self.workspaces[*ws_idx].tabs.len()
                        );
                    }
                }
                DragTarget::PaneScrollbar { pane_id, .. } => {
                    assert_live_pane(*pane_id, "pane scrollbar drag")
                }
                _ => {}
            }
        }
        for press in self.workspace_presses.values() {
            assert_workspace_index(press.ws_idx, "workspace press");
        }
        for press in self.tab_presses.values() {
            assert_tab_index(press.ws_idx, press.tab_idx, "tab press");
        }
        if let Some(menu) = self.context_menu() {
            match menu.kind {
                ContextMenuKind::Workspace { ws_idx, .. }
                | ContextMenuKind::GitWorkspace { ws_idx, .. } => {
                    assert_workspace_index(ws_idx, "context menu workspace")
                }
                ContextMenuKind::Agent {
                    ws_idx,
                    tab_idx,
                    pane_id,
                    ..
                } => {
                    assert_tab_index(ws_idx, tab_idx, "context menu agent tab");
                    assert!(
                        self.workspaces[ws_idx].tabs[tab_idx]
                            .panes
                            .contains_key(&pane_id),
                        "context menu agent references pane {:?} outside workspace {} tab {}",
                        pane_id,
                        ws_idx,
                        tab_idx
                    );
                }
                ContextMenuKind::Tab { ws_idx, tab_idx } => {
                    assert_tab_index(ws_idx, tab_idx, "context menu tab")
                }
                ContextMenuKind::Pane {
                    ws_idx,
                    tab_idx,
                    pane_id,
                    source_pane_id,
                    ..
                } => {
                    assert_tab_index(ws_idx, tab_idx, "context menu pane tab");
                    assert!(
                        self.workspaces[ws_idx].tabs[tab_idx]
                            .panes
                            .contains_key(&pane_id),
                        "context menu pane references pane {:?} outside workspace {} tab {}",
                        pane_id,
                        ws_idx,
                        tab_idx
                    );
                    if let Some(source_pane_id) = source_pane_id {
                        assert_live_pane(source_pane_id, "context menu source pane");
                    }
                }
            }
        }
    }

    pub fn insert_test_runtime(
        &mut self,
        pane_id: crate::layout::PaneId,
        runtime: crate::terminal::TerminalRuntime,
    ) {
        if let Some(ws) = self
            .workspaces
            .iter_mut()
            .find(|ws| ws.terminal_id(pane_id).is_some())
        {
            ws.insert_test_runtime(pane_id, runtime);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    #[test]
    fn pane_size_estimate_uses_the_no_client_size_while_detached() {
        let mut state = AppState::test_new();
        state.headless_size = (132, 41);
        state.view.pane_infos = vec![crate::layout::PaneInfo {
            id: crate::layout::PaneId::from_raw(1),
            rect: ratatui::layout::Rect::new(0, 0, 46, 39),
            inner_rect: ratatui::layout::Rect::new(0, 0, 46, 39),
            scrollbar_rect: None,
            borders: ratatui::widgets::Borders::NONE,
            is_focused: true,
        }];
        assert_eq!(state.estimate_pane_size(), (39, 46), "attached: the view");

        state.detached_pane_size = Some((310, 56));

        assert_eq!(state.estimate_pane_size(), (56, 310));
    }

    #[test]
    fn pane_size_estimate_uses_headless_size_before_first_view() {
        let mut state = AppState::test_new();
        state.headless_size = (132, 41);

        assert_eq!(state.estimate_pane_size(), (41, 132));
    }

    /// Builds two workspaces, each with two panes, and opens the link picker
    /// on a todo belonging to the first pane of the first workspace.
    fn state_with_link_picker_open() -> (AppState, PaneId) {
        let mut state = AppState::test_new();
        let mut first = crate::workspace::Workspace::test_new("here");
        first.test_split(ratatui::layout::Direction::Horizontal);
        let mut second = crate::workspace::Workspace::test_new("there");
        second.test_split(ratatui::layout::Direction::Horizontal);
        state.workspaces = vec![first, second];
        state.active = Some(0);
        state.ensure_test_terminals();
        let pane_id = state.workspaces[0].tabs[0].root_pane;

        state.open_new_pane_todo(pane_id);
        let runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        state.open_pane_todo_link_picker_from(&runtimes);
        (state, pane_id)
    }

    fn accept(state: &mut AppState) -> bool {
        let runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        state.accept_navigator_selection_from(&runtimes)
    }

    fn rows(state: &AppState) -> Vec<NavigatorRow> {
        let runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        state.navigator_rows_from(&runtimes)
    }

    fn select(state: &mut AppState, want: impl Fn(&NavigatorRow) -> bool) {
        state.set_navigator_selected(
            rows(state)
                .iter()
                .position(want)
                .expect("the picker should offer such a row"),
        );
    }

    /// Spec: "the navigator opens in selection mode listing panes across every
    /// workspace". The old control cycled only within the todo's own
    /// workspace, which is why most panes could never be reached.
    #[test]
    fn the_link_picker_offers_panes_from_every_workspace() {
        let (state, pane_id) = state_with_link_picker_open();
        let rows = rows(&state);

        for ws_idx in 0..2 {
            assert!(
                rows.iter().any(|row| matches!(
                    row.target,
                    NavigatorTarget::Pane { ws_idx: idx, .. } if idx == ws_idx
                )),
                "workspace {ws_idx} must contribute pane rows"
            );
        }
        assert!(
            !rows.iter().any(|row| matches!(
                row.target,
                NavigatorTarget::Pane { pane_id: candidate, .. } if candidate == pane_id
            )),
            "a todo linking to its own pane says nothing, so it is not offered"
        );
    }

    /// Spec: "when a pane row is chosen, the edit returns with that pane
    /// staged as the link target".
    #[test]
    fn choosing_a_pane_row_stages_it_and_returns_to_the_modal() {
        let (mut state, pane_id) = state_with_link_picker_open();
        select(&mut state, |row| {
            matches!(row.target, NavigatorTarget::Pane { ws_idx: 1, .. })
        });
        let NavigatorTarget::Pane {
            pane_id: target, ..
        } = rows(&state)[state.navigator_selected()].target
        else {
            panic!("selected row should be a pane");
        };

        assert!(accept(&mut state));

        assert_eq!(
            state.editing_pane_todo().expect("edit state").link,
            PaneTodoEditLink::Set(target),
            "a target in another workspace stages like any other"
        );
        assert_eq!(state.mode, Mode::PaneTodoEdit);
        assert_eq!(state.navigator_purpose(), NavigatorPurpose::Goto);
        assert_ne!(target, pane_id);
    }

    /// Spec: "non-pane rows are not targets" — a workspace row expands or
    /// collapses instead, and neither kind ends the selection.
    #[test]
    fn workspace_and_tab_rows_never_resolve_a_link() {
        let (mut state, _) = state_with_link_picker_open();
        select(&mut state, |row| row.is_workspace);
        let expanded_before = state.navigator_expanded_count();

        assert!(!accept(&mut state), "a workspace row resolves nothing");

        assert_eq!(
            state.editing_pane_todo().expect("edit state").link,
            PaneTodoEditLink::Keep,
            "the staged link is untouched"
        );
        assert_eq!(state.mode, Mode::Navigator, "the picker stays open");
        assert_ne!(
            state.navigator_expanded_count(),
            expanded_before,
            "it collapses or expands instead"
        );
    }

    #[test]
    fn the_clear_entry_clears_the_link() {
        let (mut state, _) = state_with_link_picker_open();
        select(&mut state, |row| {
            matches!(row.target, NavigatorTarget::ClearLink)
        });

        assert!(accept(&mut state));

        assert_eq!(
            state.editing_pane_todo().expect("edit state").link,
            PaneTodoEditLink::Clear
        );
        assert_eq!(state.mode, Mode::PaneTodoEdit);
    }

    /// Spec: "leaving the selection without choosing SHALL leave the link as
    /// it was".
    #[test]
    fn dismissing_the_picker_keeps_the_link_it_had() {
        let (mut state, _) = state_with_link_picker_open();
        let target = match rows(&state)
            .iter()
            .find(|row| matches!(row.target, NavigatorTarget::Pane { .. }))
            .expect("a pane row")
            .target
        {
            NavigatorTarget::Pane { pane_id, .. } => pane_id,
            _ => unreachable!(),
        };
        state.editing_pane_todo_mut().expect("edit state").link = PaneTodoEditLink::Set(target);

        state.close_pane_todo_link_picker();

        assert_eq!(
            state.editing_pane_todo().expect("edit state").link,
            PaneTodoEditLink::Set(target),
            "dismissal stages nothing, so the previous choice stands"
        );
        assert_eq!(state.mode, Mode::PaneTodoEdit);
        assert_eq!(state.navigator_purpose(), NavigatorPurpose::Goto);
    }

    /// The picker never opens on the clear entry, so a mis-keyed Enter cannot
    /// wipe a link the user meant to keep.
    #[test]
    fn the_picker_does_not_open_on_the_clear_entry() {
        let (state, _) = state_with_link_picker_open();
        assert!(state.navigator_selected() > 0);
        assert!(matches!(rows(&state)[0].target, NavigatorTarget::ClearLink));
    }

    /// Ordinary navigation must be unaffected: no clear entry, every pane
    /// offered, and Enter still focuses.
    #[test]
    fn the_goto_navigator_is_unchanged_by_the_picker() {
        let mut state = AppState::test_new();
        state.workspaces = vec![crate::workspace::Workspace::test_new("here")];
        state.active = Some(0);
        state.ensure_test_terminals();
        state.open_navigator();

        assert_eq!(state.navigator_purpose(), NavigatorPurpose::Goto);
        assert!(!rows(&state)
            .iter()
            .any(|row| matches!(row.target, NavigatorTarget::ClearLink)));

        assert!(accept(&mut state));
        assert_eq!(state.mode, Mode::Terminal, "goto still focuses and closes");
    }

    fn test_toast(kind: ToastKind, title: &str, target: Option<ToastTarget>) -> ToastNotification {
        ToastNotification {
            kind,
            title: title.to_string(),
            context: "ctx".to_string(),
            position: None,
            target,
            anchor_pane: None,
        }
    }

    #[test]
    fn notification_center_mode_wants_ascii_input() {
        assert!(Mode::NotificationCenter.wants_ascii_input());
    }

    #[test]
    fn notification_log_assigns_monotonic_ids_and_evicts_beyond_capacity() {
        let mut log = NotificationLog::default();
        for i in 0..NOTIFICATION_LOG_CAPACITY + 5 {
            log.post(
                &test_toast(ToastKind::Finished, &format!("toast {i}"), None),
                i as u64,
            );
        }

        assert_eq!(log.len(), NOTIFICATION_LOG_CAPACITY);
        let ids: Vec<u64> = log.entries_newest_first().map(|entry| entry.id).collect();
        assert_eq!(ids.first(), Some(&(NOTIFICATION_LOG_CAPACITY as u64 + 5)));
        assert_eq!(ids.last(), Some(&6), "oldest entries evicted");
        assert!(
            ids.windows(2).all(|pair| pair[0] == pair[1] + 1),
            "ids strictly descending newest-first: {ids:?}"
        );
    }

    #[test]
    fn notification_log_unread_tracks_read_flags_and_mark_seen_is_idempotent() {
        let mut log = NotificationLog::default();
        log.post(&test_toast(ToastKind::Finished, "one", None), 1);
        assert_eq!(log.unread_count(), 1);
        assert!(log.mark_all_seen());
        assert_eq!(log.unread_count(), 0);

        log.post(&test_toast(ToastKind::NeedsAttention, "two", None), 2);
        log.post(&test_toast(ToastKind::UpdateInstalled, "three", None), 3);
        assert_eq!(log.unread_count(), 2);

        assert!(log.mark_all_seen());
        assert!(!log.mark_all_seen(), "second mark_seen is a no-op");
        assert_eq!(log.unread_count(), 0);
    }

    #[test]
    fn notification_log_mark_read_quiets_exactly_one_entry() {
        let mut log = NotificationLog::default();
        let one = log.post(&test_toast(ToastKind::Finished, "one", None), 1);
        let two = log.post(&test_toast(ToastKind::NeedsAttention, "two", None), 2);
        log.post(&test_toast(ToastKind::UpdateInstalled, "three", None), 3);
        assert_eq!(log.unread_count(), 3);

        assert!(log.mark_read(two));
        assert_eq!(log.unread_count(), 2);
        assert!(!log.mark_read(two), "second mark_read is a no-op");
        assert!(!log.mark_read(9999), "unknown ids change nothing");
        assert_eq!(log.unread_count(), 2);

        let read_flags: Vec<(u64, bool)> = log
            .entries_newest_first()
            .map(|entry| (entry.id, entry.read))
            .collect();
        assert!(
            read_flags.contains(&(two, true)) && read_flags.contains(&(one, false)),
            "only the marked entry is read: {read_flags:?}"
        );
    }

    #[test]
    fn notification_log_clear_empties_entries_and_keeps_ids_monotonic() {
        let mut log = NotificationLog::default();
        log.post(&test_toast(ToastKind::Finished, "one", None), 1);
        log.post(&test_toast(ToastKind::Finished, "two", None), 2);
        log.mark_all_seen();

        assert_eq!(log.clear(), 2, "clear reports the number removed");
        assert!(log.is_empty());
        assert_eq!(log.unread_count(), 0);

        // Ids never rewind: the next post continues past the cleared entries.
        let next_id = log.post(&test_toast(ToastKind::Finished, "three", None), 3);
        assert_eq!(next_id, 3);
        assert_eq!(log.unread_count(), 1, "the post-clear entry is unread");
    }

    #[test]
    fn clear_notifications_empties_log_and_resets_panel_selection() {
        let mut state = AppState::test_new();
        for title in ["a", "b", "c"] {
            state.post_notification(test_toast(ToastKind::Finished, title, None));
        }
        state.open_notification_center();
        state.notification_center_move_selection(2);
        assert_eq!(
            state.notification_center().map(|c| c.list.selected),
            Some(2)
        );

        state.clear_notifications();

        assert!(state.notification_log.is_empty());
        assert_eq!(
            state.mode,
            Mode::NotificationCenter,
            "clearing leaves the panel open"
        );
        assert_eq!(
            state.notification_center().map(|c| c.list.selected),
            Some(0),
            "selection resets after clear"
        );
    }

    #[test]
    fn notification_center_footer_splits_the_list_and_button_rows() {
        let mut state = AppState::test_new();
        state.view.terminal_area = Rect::new(0, 1, 80, 24);
        state.view.tab_bar_rect = Rect::new(0, 0, 80, 1);
        for title in ["a", "b", "c"] {
            state.post_notification(test_toast(ToastKind::Finished, title, None));
        }
        state.open_notification_center();

        let buttons = state
            .notification_center_buttons()
            .expect("footer buttons present with entries");
        let button = buttons
            .rect(NotificationCenterButton::Clear)
            .expect("clear all is never dropped");
        let close = buttons
            .rect(NotificationCenterButton::Close)
            .expect("close is never dropped");
        let (list, _start) = state
            .notification_center_list_window()
            .expect("list window present");

        // One blank row separates the last entry from the buttons — the panel
        // convention, so nothing sits flush against the footer.
        assert_eq!(button.height, 1);
        assert_eq!(
            list.y + list.height + 1,
            button.y,
            "one blank row between the list and the buttons"
        );
        assert_eq!(close.y, button.y, "buttons share the footer row");
        assert_eq!(list.height, 3);
        assert!(button.width <= list.width, "button fits within the panel");
        assert!(button.x >= list.x, "button sits within the inner area");
        assert!(
            close.x + close.width <= list.x + list.width,
            "buttons stay within the inner area"
        );
    }

    /// The regression this fixes: at the panel's old 30-column minimum the
    /// footer wanted 41, so `mark read` was dropped and the `r` hint went with
    /// it — the action was reachable only by a key nothing advertised.
    #[test]
    fn the_notification_panel_is_never_narrower_than_its_own_footer() {
        let mut state = AppState::test_new();
        state.view.terminal_area = Rect::new(0, 1, 80, 24);
        state.view.tab_bar_rect = Rect::new(0, 0, 80, 1);
        // One short title: nothing about the entries wants a wide panel.
        state.post_notification(test_toast(ToastKind::Finished, "ok", None));
        state.open_notification_center();

        let rect = state.notification_center_rect().expect("panel rect");
        assert!(
            rect.width >= crate::ui::notification_center_footer_width() + 2,
            "panel {} is narrower than its footer",
            rect.width
        );

        let buttons = state
            .notification_center_buttons()
            .expect("footer buttons present with entries");
        assert!(
            buttons.rect(NotificationCenterButton::MarkRead).is_some(),
            "mark read must survive at the panel's own width"
        );
    }

    /// An empty log has no footer to fit, so it stays the narrow box it was.
    #[test]
    fn an_empty_notification_panel_does_not_widen_for_a_footer_it_has_not_got() {
        let mut state = AppState::test_new();
        state.view.terminal_area = Rect::new(0, 1, 80, 24);
        state.view.tab_bar_rect = Rect::new(0, 0, 80, 1);
        state.open_notification_center();

        let rect = state.notification_center_rect().expect("panel rect");
        assert_eq!(rect.width, 30);
        assert!(state.notification_center_buttons().is_none());
    }

    #[test]
    fn notification_center_rect_honors_bottom_right_position() {
        let mut state = AppState::test_new();
        state.view.terminal_area = Rect::new(0, 1, 80, 24);
        state.view.tab_bar_rect = Rect::new(0, 0, 80, 1);
        state.post_notification(test_toast(ToastKind::Finished, "one", None));
        state.open_notification_center();

        let top = state.notification_center_rect().expect("top-right rect");
        assert_eq!(top.y, 1, "top-right anchors under the tab bar");

        state.notification_center_position =
            crate::config::NotificationCenterPositionConfig::BottomRight;
        let bottom = state.notification_center_rect().expect("bottom-right rect");
        assert_eq!(
            bottom.y + bottom.height,
            25,
            "without a floating indicator the panel sits at the screen bottom"
        );
        assert_eq!(bottom.x, top.x, "right alignment is unchanged");
        assert_eq!(bottom.width, top.width);
        assert_eq!(bottom.height, top.height);

        // With the floating indicator on the frame's last row (as compute_view
        // sets it for bottom-right), the panel opens directly above it so the
        // diamond stays visible as the toggle.
        state.view.notification_hit_area = Rect::new(75, 24, 5, 1);
        let above = state
            .notification_center_rect()
            .expect("indicator-anchored rect");
        assert_eq!(
            above.y + above.height,
            24,
            "panel bottom sits on top of the indicator row"
        );
    }

    #[test]
    fn notification_center_has_no_footer_button_when_empty() {
        let mut state = AppState::test_new();
        state.view.terminal_area = Rect::new(0, 1, 80, 24);
        state.view.tab_bar_rect = Rect::new(0, 0, 80, 1);
        state.open_notification_center();

        assert!(state.notification_log.is_empty());
        assert!(state.notification_center_buttons().is_none());
    }

    #[test]
    fn post_notification_shows_toast_and_appends_log_entry() {
        let mut state = AppState::test_new();
        state.post_notification(test_toast(ToastKind::Finished, "claude finished", None));

        assert_eq!(
            state.toast.as_ref().map(|toast| toast.title.as_str()),
            Some("claude finished")
        );
        let entry = state
            .notification_log
            .entries_newest_first()
            .next()
            .cloned()
            .expect("entry logged");
        assert_eq!(entry.title, "claude finished");
        assert_eq!(entry.kind, ToastKind::Finished);
        let pending = state.notification_log.take_pending_events();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, entry.id);
        assert!(state.notification_log.take_pending_events().is_empty());
    }

    #[test]
    fn open_notification_center_keeps_unread_and_clamps_selection() {
        let mut state = AppState::test_new();
        for title in ["one", "two", "three"] {
            state.post_notification(test_toast(ToastKind::Finished, title, None));
        }
        assert_eq!(state.notification_log.unread_count(), 3);

        state.open_notification_center();
        assert_eq!(state.mode, Mode::NotificationCenter);
        assert_eq!(
            state.notification_log.unread_count(),
            3,
            "opening the panel leaves read state alone"
        );
        assert_eq!(
            state
                .notification_center()
                .map(|center| center.list.selected),
            Some(0)
        );

        state.notification_center_move_selection(1);
        state.notification_center_move_selection(10);
        assert_eq!(
            state
                .notification_center()
                .map(|center| center.list.selected),
            Some(2),
            "selection clamps to newest-first list length"
        );
        state.notification_center_move_selection(-10);
        assert_eq!(
            state
                .notification_center()
                .map(|center| center.list.selected),
            Some(0)
        );
        assert_eq!(
            state
                .notification_center_selected_entry()
                .map(|entry| entry.title.as_str()),
            Some("three"),
            "selection 0 is the newest entry"
        );
    }

    #[test]
    fn agent_terminal_keeps_final_child_cursor_exposed() {
        let mut state = AppState::test_new();
        let ws = crate::workspace::Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        state.terminals.insert(
            ws.tabs[0].panes[&pane_id].attached_terminal_id.clone(),
            crate::terminal::TerminalState::new(
                ws.tabs[0].panes[&pane_id].attached_terminal_id.clone(),
                std::path::PathBuf::from("/tmp"),
            ),
        );
        state
            .terminals
            .get_mut(&ws.tabs[0].panes[&pane_id].attached_terminal_id)
            .expect("terminal state")
            .launch_argv = Some(vec!["codex".to_string()]);
        state.workspaces = vec![ws];

        assert!(state.pane_exposes_host_cursor(0, pane_id));
    }

    #[test]
    fn adversarial_identity_state_satisfies_app_invariants_after_mutation() {
        let mut state = AppState::test_with_adversarial_identity_state();
        state.assert_invariants_for_test();

        let ws = &mut state.workspaces[0];
        let active_public = ws.tabs[ws.active_tab].number;
        assert_ne!(ws.active_tab + 1, active_public);
        let new_pane = ws.test_split(ratatui::layout::Direction::Horizontal);
        assert!(ws.public_pane_number(new_pane).is_some());
        state.ensure_test_terminals();

        state.assert_invariants_for_test();
    }

    /// Every overlay, opened over adversarial identity state, agrees with the
    /// mode — and closing it leaves nothing behind that names an overlay.
    #[test]
    fn every_overlay_agrees_with_the_mode_over_adversarial_identity_state() {
        for kind in OverlayKind::ALL {
            let mut state = AppState::test_with_adversarial_identity_state();
            let pane_id = state.workspaces[0].tabs[0].root_pane;
            state.open_overlay(overlay_for_kind(*kind, pane_id));
            assert_eq!(state.open_overlay_kind(), Some(*kind));
            state.assert_invariants_for_test();

            state.close_any_overlay();
            state.mode = Mode::Terminal;
            state.assert_invariants_for_test();
        }
    }

    /// A mode that names an overlay with no overlay open is exactly the
    /// disagreement the enum exists to prevent, so the invariant must catch it.
    #[test]
    fn a_mode_naming_a_closed_overlay_fails_the_invariants() {
        for kind in OverlayKind::ALL {
            let mut state = AppState::test_with_adversarial_identity_state();
            state.mode = kind.mode();
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                state.assert_invariants_for_test()
            }));
            assert!(
                caught.is_err(),
                "{kind:?}'s mode with no overlay open should fail the invariants"
            );
        }
    }

    /// And the other direction: an overlay open under someone else's mode.
    #[test]
    fn an_overlay_under_the_wrong_mode_fails_the_invariants() {
        let mut state = AppState::test_with_adversarial_identity_state();
        let pane_id = state.workspaces[0].tabs[0].root_pane;
        state.open_overlay(overlay_for_kind(OverlayKind::PaneTodos, pane_id));
        state.mode = Mode::Settings;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.assert_invariants_for_test()
        }));
        assert!(
            caught.is_err(),
            "mode and overlay must not be allowed to disagree"
        );
    }

    fn test_worktree_create_state() -> WorktreeCreateState {
        WorktreeCreateState {
            source_workspace_id: "ws".into(),
            source_checkout_path: std::path::PathBuf::from("/tmp/repo"),
            source_existing_membership: None,
            source_repo_root: std::path::PathBuf::from("/tmp/repo"),
            repo_key: "repo".into(),
            repo_name: "repo".into(),
            branch: "issue/1".into(),
            checkout_path: std::path::PathBuf::from("/tmp/repo-issue-1"),
            error: None,
            creating: false,
        }
    }

    fn test_worktree_open_state() -> WorktreeOpenState {
        WorktreeOpenState {
            source_workspace_id: "ws".into(),
            source_existing_membership: None,
            source_checkout_path: std::path::PathBuf::from("/tmp/repo"),
            source_repo_root: std::path::PathBuf::from("/tmp/repo"),
            repo_key: "repo".into(),
            repo_name: "repo".into(),
            entries: Vec::new(),
            selected: 0,
            query: String::new(),
            search_focused: false,
            error: None,
        }
    }

    fn test_worktree_remove_state() -> WorktreeRemoveState {
        WorktreeRemoveState {
            workspace_id: "ws".into(),
            repo_root: std::path::PathBuf::from("/tmp/repo"),
            path: std::path::PathBuf::from("/tmp/repo-issue-1"),
            error: None,
            removing: false,
            force_confirmation: false,
        }
    }

    /// One representative state per overlay. Exhaustive on purpose: a new
    /// overlay does not compile until the invariant tests can open it.
    fn overlay_for_kind(kind: OverlayKind, pane_id: PaneId) -> Overlay {
        match kind {
            OverlayKind::Settings => Overlay::Settings(SettingsState {
                section: SettingsSection::Theme,
                list: ListCursor::new(0),
                original_palette: None,
                original_theme: None,
            }),
            OverlayKind::GlobalMenu => Overlay::GlobalMenu(ListCursor::new(0)),
            OverlayKind::KeybindHelp => Overlay::KeybindHelp(KeybindHelpState::default()),
            OverlayKind::Navigator => Overlay::Navigator(NavigatorState::default()),
            OverlayKind::ContextMenu => Overlay::ContextMenu(ContextMenuState {
                kind: ContextMenuKind::Workspace {
                    ws_idx: 0,
                    pinned: false,
                },
                x: 0,
                y: 0,
                list: ListCursor::new(0),
            }),
            OverlayKind::NotificationCenter => {
                Overlay::NotificationCenter(NotificationCenterState {
                    list: ListCursor::default(),
                    hovered_button: None,
                })
            }
            OverlayKind::PaneTodos => Overlay::PaneTodos(PaneTodoPanelState {
                pane_id,
                list: ListCursor::default(),
                hovered_button: None,
            }),
            OverlayKind::PaneTodoEdit => Overlay::PaneTodoEdit(PaneTodoEditState {
                pane_id,
                todo_id: None,
                text: crate::ui::text_field::TextField::new(
                    crate::terminal::todo::MAX_TODO_TEXT_LEN,
                ),
                priority: crate::terminal::todo::TodoPriority::default(),
                link: PaneTodoEditLink::Keep,
                done: false,
                suspended: None,
            }),
            OverlayKind::ReleaseNotes => Overlay::ReleaseNotes(ReleaseNotesState {
                version: "0.0.0".into(),
                body: String::new(),
                scroll: 0,
                preview: false,
            }),
            OverlayKind::ProductAnnouncement => {
                Overlay::ProductAnnouncement(ProductAnnouncementState {
                    version: "0.0.0".into(),
                    id: "id".into(),
                    title: String::new(),
                    body: String::new(),
                    scroll: 0,
                    preview: false,
                })
            }
            OverlayKind::NewLinkedWorktree => {
                Overlay::NewLinkedWorktree(test_worktree_create_state())
            }
            OverlayKind::OpenExistingWorktree => {
                Overlay::OpenExistingWorktree(test_worktree_open_state())
            }
            OverlayKind::ConfirmRemoveWorktree => {
                Overlay::ConfirmRemoveWorktree(test_worktree_remove_state())
            }
            OverlayKind::PaneMoveTargetPicker => Overlay::PaneMoveTargetPicker(
                PaneMoveTargetPickerState::new("p1".into(), Vec::new()),
            ),
            OverlayKind::DisplayPanes => Overlay::DisplayPanes(DisplayPanesState {
                deadline: std::time::Instant::now(),
            }),
            OverlayKind::TodoBoard => Overlay::TodoBoard(TodoBoardState::new(vec![
                TodoBoardItem::PaneHeading {
                    space: "one".into(),
                    label: "shell".into(),
                },
                TodoBoardItem::Todo {
                    pane_id,
                    todo_id: 1,
                },
            ])),
        }
    }

    fn navigator_row_for_display(is_workspace: bool) -> NavigatorRow {
        NavigatorRow {
            target: NavigatorTarget::Workspace { ws_idx: 0 },
            depth: if is_workspace { 0 } else { 1 },
            label: String::new(),
            meta: String::new(),
            status: crate::detect::AgentState::Idle,
            seen: true,
            is_current: false,
            is_workspace,
            is_tab: false,
            expanded: true,
            public_pane_id: None,
            search_text: String::new(),
            matched: true,
        }
    }

    #[test]
    fn navigator_display_lines_separate_workspace_groups() {
        let rows = vec![
            navigator_row_for_display(true),
            navigator_row_for_display(false),
            navigator_row_for_display(true),
            navigator_row_for_display(false),
        ];
        assert_eq!(
            navigator_display_lines(&rows),
            vec![
                NavigatorDisplayLine::Row(0),
                NavigatorDisplayLine::Row(1),
                NavigatorDisplayLine::Spacer,
                NavigatorDisplayLine::Row(2),
                NavigatorDisplayLine::Row(3),
            ]
        );
    }

    #[test]
    fn navigator_display_lines_have_no_leading_spacer() {
        let rows = vec![
            navigator_row_for_display(true),
            navigator_row_for_display(false),
        ];
        assert_eq!(
            navigator_display_lines(&rows),
            vec![NavigatorDisplayLine::Row(0), NavigatorDisplayLine::Row(1)]
        );
        assert!(navigator_display_lines(&[]).is_empty());
    }

    #[test]
    fn navigator_display_index_maps_row_to_line() {
        let rows = vec![
            navigator_row_for_display(true),
            navigator_row_for_display(false),
            navigator_row_for_display(true),
        ];
        let lines = navigator_display_lines(&rows);
        assert_eq!(navigator_display_index_of_row(&lines, 2), Some(3));
        assert_eq!(navigator_display_index_of_row(&lines, 9), None);
    }

    #[test]
    fn navigator_first_row_skips_spacer_lines() {
        let rows = vec![
            navigator_row_for_display(true),
            navigator_row_for_display(false),
            navigator_row_for_display(true),
        ];
        let lines = navigator_display_lines(&rows);
        // Line 2 is the spacer before the second workspace.
        assert_eq!(navigator_first_row_at_or_after(&lines, 2), Some(2));
        assert_eq!(navigator_first_row_at_or_after(&lines, 4), None);
    }

    fn rgb_luminance(color: Color) -> f64 {
        let Color::Rgb(r, g, b) = color else {
            panic!("expected RGB color, got {color:?}");
        };
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    fn contrast_ratio(a: Color, b: Color) -> f64 {
        let (lighter, darker) = {
            let a = rgb_luminance(a);
            let b = rgb_luminance(b);
            (a.max(b), a.min(b))
        };
        (lighter + 0.05) / (darker + 0.05)
    }

    #[test]
    fn built_in_theme_names_resolve() {
        for name in THEME_NAMES {
            assert!(
                Palette::from_name(name).is_some(),
                "theme should resolve: {name}"
            );
        }
    }

    #[test]
    fn built_in_active_rows_remain_visible_with_matching_terminal_backgrounds() {
        for name in THEME_NAMES
            .iter()
            .copied()
            .filter(|name| *name != "terminal")
        {
            let palette = Palette::from_name(name).unwrap();
            let background_contrast = contrast_ratio(palette.panel_bg, palette.active_row_bg);
            assert!(
                background_contrast >= 1.05,
                "active row blends into the matching terminal background for {name}: {background_contrast:.2}:1"
            );

            let text_contrast = contrast_ratio(palette.text, palette.active_row_bg);
            assert!(
                text_contrast >= 3.0,
                "active row text loses contrast for {name}: {text_contrast:.2}:1"
            );
        }
    }

    #[test]
    fn built_in_selection_rows_stay_distinct_from_background_and_active_rows() {
        for name in THEME_NAMES
            .iter()
            .copied()
            .filter(|name| *name != "terminal")
        {
            let palette = Palette::from_name(name).unwrap();
            let background_contrast = contrast_ratio(palette.panel_bg, palette.selection_bg);
            assert!(
                background_contrast >= 1.05,
                "selection row blends into the matching terminal background for {name}: {background_contrast:.2}:1"
            );

            let text_contrast = contrast_ratio(palette.text, palette.selection_bg);
            assert!(
                text_contrast >= 3.0,
                "selection row text loses contrast for {name}: {text_contrast:.2}:1"
            );
            assert_ne!(
                palette.selection_bg, palette.active_row_bg,
                "selection row shares the active row color for {name}"
            );
        }
    }

    #[test]
    fn built_in_themes_leave_sidebar_background_unset() {
        for name in THEME_NAMES {
            let palette = Palette::from_name(name).unwrap();
            assert_eq!(
                palette.sidebar_bg,
                Color::Reset,
                "built-in theme changed the sidebar background: {name}"
            );
        }
    }

    #[test]
    fn custom_sidebar_colors_override_the_defaults() {
        let custom = crate::config::CustomThemeColors {
            sidebar_bg: Some("#181825".to_string()),
            active_row_bg: Some("#313244".to_string()),
            selection_bg: Some("#45475a".to_string()),
            ..Default::default()
        };
        let palette = Palette::catppuccin().with_overrides(&custom);

        assert_eq!(palette.sidebar_bg, Color::Rgb(24, 24, 37));
        assert_eq!(palette.active_row_bg, Color::Rgb(49, 50, 68));
        assert_eq!(palette.selection_bg, Color::Rgb(69, 71, 90));
    }

    #[test]
    fn light_theme_aliases_resolve() {
        for name in ["light", "latte", "tokyo-day", "onelight", "lotus", "dawn"] {
            assert!(
                Palette::from_name(name).is_some(),
                "theme should resolve: {name}"
            );
        }
    }

    #[test]
    fn key_matches_requires_exact_modifiers() {
        assert!(key_matches(
            &KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        ));

        assert!(!key_matches(
            &KeyEvent::new(
                KeyCode::Char('b'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        ));
    }

    #[test]
    fn key_matches_letters_case_insensitively() {
        assert!(key_matches(
            &KeyEvent::new(KeyCode::Char('B'), KeyModifiers::SHIFT),
            KeyCode::Char('b'),
            KeyModifiers::SHIFT,
        ));
    }

    #[test]
    fn linked_worktree_context_menu_keeps_safe_close_and_explicit_remove() {
        let menu = ContextMenuState {
            kind: ContextMenuKind::GitWorkspace {
                ws_idx: 0,
                pinned: false,
                is_linked_worktree: true,
                has_worktree_children: false,
                collapsed: false,
            },
            x: 0,
            y: 0,
            list: ListCursor::new(0),
        };

        assert_eq!(
            menu.items(),
            &["Pin", "Rename", "Close", "Delete worktree checkout..."]
        );
    }

    #[test]
    fn git_workspace_context_menu_keeps_remove_for_managed_worktrees_only() {
        let menu = ContextMenuState {
            kind: ContextMenuKind::GitWorkspace {
                ws_idx: 0,
                pinned: false,
                is_linked_worktree: false,
                has_worktree_children: false,
                collapsed: false,
            },
            x: 0,
            y: 0,
            list: ListCursor::new(0),
        };

        assert_eq!(
            menu.items(),
            &["Pin", "Rename", "Close", "New worktree", "Open worktree..."]
        );
    }

    #[test]
    fn parent_worktree_context_menu_uses_repo_actions() {
        let menu = ContextMenuState {
            kind: ContextMenuKind::GitWorkspace {
                ws_idx: 0,
                pinned: false,
                is_linked_worktree: false,
                has_worktree_children: true,
                collapsed: false,
            },
            x: 0,
            y: 0,
            list: ListCursor::new(0),
        };

        assert_eq!(
            menu.items(),
            &[
                "Pin",
                "Rename",
                "Close group",
                "New worktree",
                "Open worktree...",
                "Collapse"
            ]
        );
    }
}
