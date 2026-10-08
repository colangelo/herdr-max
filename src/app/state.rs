use crate::config::{Keybinds, NewTerminalCwdConfig, SoundConfig, ToastConfig};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::detect::AgentState;
use crate::layout::{PaneId, PaneInfo};

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

use crate::terminal_theme::{HostAppearance, TerminalTheme};
use crate::workspace::Workspace;

// ---------------------------------------------------------------------------
// Theme palette — all UI colors in one place, ready for theming
// ---------------------------------------------------------------------------

/// All colors used by the UI. Derived from a base accent color for now,
/// but structured so a full theme system can replace it later.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Geometry for the server-rendered active-tab pane surface.
pub struct ViewState {
    pub terminal_area: Rect,
    pub pane_infos: Vec<PaneInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Navigate,
    Terminal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentPanelSort {
    #[default]
    Spaces,
    Priority,
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

#[derive(Debug)]
pub struct ReleaseNotesState {
    pub version: String,
    pub body: String,
    pub scroll: u16,
    pub preview: bool,
}

#[derive(Debug)]
pub struct ProductAnnouncementState {
    pub version: String,
    pub id: String,
    pub title: String,
    pub body: String,
    pub scroll: u16,
    pub preview: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneFocusTarget {
    pub workspace_id: String,
    pub pane_id: PaneId,
}

/// Endpoint-configured pane appearance; resolved at startup/reload, pure during rendering.
#[derive(Clone, Copy)]
pub(crate) struct PaneAppearance {
    pub(crate) show_todos: bool,
    pub(crate) todo_color: Option<Color>,
    pub(crate) border_active: Option<Color>,
    pub(crate) border_inactive: Option<Color>,
    pub(crate) border_style: crate::config::PaneBorderActiveStyleConfig,
    pub(crate) title_active: Option<Color>,
    pub(crate) title_inactive: Option<Color>,
    pub(crate) active_bg: Option<Color>,
    pub(crate) inactive_bg: Option<Color>,
    pub(crate) dim_inactive: bool,
    pub(crate) inactive_dim: u32,
}

impl Default for PaneAppearance {
    fn default() -> Self {
        Self {
            show_todos: true,
            todo_color: None,
            border_active: None,
            border_inactive: None,
            border_style: crate::config::PaneBorderActiveStyleConfig::Light,
            title_active: None,
            title_inactive: None,
            active_bg: None,
            inactive_bg: None,
            dim_inactive: false,
            inactive_dim: 0,
        }
    }
}

impl PaneAppearance {
    pub(crate) fn from_config(config: &crate::config::Config) -> Self {
        let ui = &config.ui;
        let parse = |value: &Option<String>| value.as_deref().map(crate::config::parse_color);
        Self {
            show_todos: ui.show_pane_todo_indicator,
            todo_color: parse(&ui.pane_todo_color),
            border_active: parse(&ui.pane_border_active_color),
            border_inactive: parse(&ui.pane_border_inactive_color),
            border_style: ui.pane_border_active_style,
            title_active: parse(&ui.pane_title_active_color),
            title_inactive: parse(&ui.pane_title_inactive_color),
            active_bg: parse(&ui.pane_active_bg),
            inactive_bg: parse(&ui.pane_inactive_bg),
            dim_inactive: ui.dim_inactive_panes,
            inactive_dim: config.inactive_pane_dim(),
        }
    }
    pub(crate) fn border_color(&self, focused: bool, palette: &Palette) -> Color {
        if focused {
            self.border_active.unwrap_or(palette.accent)
        } else {
            self.border_inactive.unwrap_or(palette.overlay0)
        }
    }
    pub(crate) fn title_color(&self, focused: bool, palette: &Palette) -> Color {
        (if focused {
            self.title_active
        } else {
            self.title_inactive
        })
        .unwrap_or_else(|| self.border_color(focused, palette))
    }
}

/// All application state — pure data, no channels or async runtime.
/// Testable without PTYs or a tokio runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabBarStatusSegment {
    Zoom,
    Text(Option<String>),
}

pub struct AppState {
    pub(crate) pane_appearance: PaneAppearance,
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
    pub should_quit: bool,
    /// Set when the headless server should ask attached clients to reload
    /// their client-local sound config from disk.
    pub request_client_config_reload: bool,
    pub worktree_directory: std::path::PathBuf,
    /// Latest endpoint-owned release notes, cached outside render paths.
    pub latest_release_notes: Option<crate::release_notes::ReleaseNotes>,
    pub product_announcement: Option<ProductAnnouncementState>,
    // Geometry of the most recently computed server pane surface.
    pub view: ViewState,
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
    /// Last reported focus state for the outer terminal hosting herdr.
    /// None means unsupported or not yet reported, which preserves active-pane suppression.
    pub outer_terminal_focus: Option<bool>,
    // Config
    pub prefix_keys: Vec<(KeyCode, KeyModifiers)>,
    /// Virtual terminal size (columns, rows) used when no client is attached.
    pub(crate) headless_size: (u16, u16),
    /// The size, (cols, rows), of the last foreground client that was at least
    /// [`REMEMBERED_CLIENT_SIZE_FLOOR`]. The session file keeps it, so it
    /// outlives the client and the server.
    pub(crate) last_client_size: Option<(u16, u16)>,
    /// `server.remember_client_size`: whether the last client size is kept
    /// and used as the no-client size.
    pub(crate) remember_client_size: bool,
    pub agent_panel_sort: AgentPanelSort,
    /// Transient session-wide projection override for the built-in Agents view.
    pub agent_view_override: Option<crate::api::schema::AgentViewSetParams>,
    pub sidebar_agents: crate::config::AgentsSidebarConfig,
    pub sidebar_spaces: crate::config::SpacesSidebarConfig,
    pub workspace_sort: WorkspaceSort,
    pub next_agent_state_change_seq: u64,
    pub confirm_close: bool,
    pub pane_borders: crate::config::PaneBordersConfig,
    pub pane_outer_borders: bool,
    pub pane_scrollbars: bool,
    pub pane_gaps: bool,
    pub show_agent_labels_on_pane_borders: bool,
    pub tab_bar_right: Vec<TabBarStatusSegment>,
    pub tab_bar_right_separator: String,
    /// Expose the focused pane's cursor anchor to the outer terminal even when
    /// the pane requested `?25l`. See `[experimental] reveal_hidden_cursor_for_cjk_ime`.
    pub reveal_hidden_cursor_for_cjk_ime: bool,
    /// Restrict cursor reveal to focused panes whose detected agent matches
    /// one of these. When false, apply to any focused pane.
    pub cjk_ime_agent_filter_configured: bool,
    pub cjk_ime_agents: Vec<crate::detect::Agent>,
    /// DECSCUSR shape parameter (1–6) for the IME anchor cursor.
    pub cjk_ime_cursor_shape: u8,
    pub kitty_graphics_enabled: bool,
    pub default_shell: String,
    pub shell_mode: crate::config::ShellModeConfig,
    pub new_terminal_cwd: NewTerminalCwdConfig,
    pub pane_scrollback_limit_bytes: usize,
    pub sound: SoundConfig,
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
    /// Cached integration recommendations and detection manifest summaries.
    pub integration_recommendations: Vec<crate::integration::IntegrationRecommendation>,
    pub agent_manifest_summaries: Vec<crate::detect::manifest::AgentManifestSummary>,
    /// Cached remote detection manifest update diagnostics for runtime/API status.
    pub agent_manifest_update_status: crate::detect::manifest_update::ManifestUpdateStatus,
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
    /// Resolved host terminal default colors for theming embedded panes.
    pub host_terminal_theme: TerminalTheme,
    /// Last known foreground host terminal cell size in pixels.
    pub(crate) host_cell_size: crate::kitty_graphics::HostCellSize,
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

    pub(crate) fn pane_exposes_host_cursor(
        &self,
        _ws_idx: usize,
        _pane_id: crate::layout::PaneId,
    ) -> bool {
        true
    }

    pub(crate) fn refresh_agent_manifest_summaries(&mut self) {
        self.agent_manifest_summaries = crate::detect::manifest::manifest_summaries();
    }

    pub(crate) fn integration_updates_available(&self) -> bool {
        self.integration_recommendations
            .iter()
            .any(|recommendation| {
                recommendation.state == crate::integration::IntegrationStatusKind::Outdated
                    && recommendation.needs_install()
            })
    }

    /// Rows and columns a pane being created at `placement` will have once its
    /// tab is laid out, including tabs that are not on screen.
    pub(crate) fn new_pane_size(&self, placement: crate::ui::NewPanePlacement) -> (u16, u16) {
        crate::ui::new_pane_terminal_size(self, self.new_pane_area(), placement)
    }

    /// Rows and columns for every pane of a tab about to be built as `layout`,
    /// in pane order.
    pub(crate) fn new_layout_pane_sizes(
        &self,
        layout: &crate::layout::TileLayout,
    ) -> Vec<(u16, u16)> {
        crate::ui::new_layout_terminal_sizes(self, self.new_pane_area(), layout)
    }

    fn new_pane_area(&self) -> Rect {
        let area = self.view.terminal_area;
        if area.width == 0 || area.height == 0 {
            Rect::new(0, 0, self.headless_size.0, self.headless_size.1)
        } else {
            area
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

    pub(crate) fn pane_visible_on_active_surface(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> bool {
        if self.active != Some(ws_idx) {
            return false;
        }
        let Some(tab) = self
            .workspaces
            .get(ws_idx)
            .and_then(|workspace| workspace.active_tab())
        else {
            return false;
        };
        if tab.zoomed {
            tab.layout.focused() == pane_id
        } else {
            tab.layout.pane_ids().contains(&pane_id)
        }
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
            should_quit: false,
            request_client_config_reload: false,
            worktree_directory: std::path::PathBuf::from("/tmp/herdr-worktrees"),
            latest_release_notes: None,
            product_announcement: None,
            view: ViewState {
                terminal_area: Rect::default(),
                pane_infos: Vec::new(),
            },
            update_available: None,
            update_install_command: "herdr update".into(),
            latest_release_notes_available: false,
            update_dismissed: false,
            config_diagnostic: None,
            toast: None,
            notification_log: NotificationLog::default(),
            pending_agent_notifications: std::collections::HashMap::new(),
            outer_terminal_focus: None,
            prefix_keys: vec![(KeyCode::Char('b'), KeyModifiers::CONTROL)],
            headless_size: (
                crate::config::DEFAULT_HEADLESS_COLS,
                crate::config::DEFAULT_HEADLESS_ROWS,
            ),
            last_client_size: None,
            remember_client_size: true,
            agent_panel_sort: AgentPanelSort::Spaces,
            agent_view_override: None,
            sidebar_agents: crate::config::AgentsSidebarConfig::default(),
            sidebar_spaces: crate::config::SpacesSidebarConfig::default(),
            workspace_sort: WorkspaceSort::Manual,
            next_agent_state_change_seq: 0,
            confirm_close: true,
            pane_appearance: PaneAppearance::default(),
            pane_borders: crate::config::PaneBordersConfig::Auto,
            pane_outer_borders: true,
            pane_scrollbars: true,
            pane_gaps: false,
            show_agent_labels_on_pane_borders: false,
            tab_bar_right: Vec::new(),
            tab_bar_right_separator: " ".into(),
            reveal_hidden_cursor_for_cjk_ime: false,
            cjk_ime_agent_filter_configured: false,
            cjk_ime_agents: Vec::new(),
            cjk_ime_cursor_shape: 2, // steady_block
            kitty_graphics_enabled: false,
            default_shell: String::new(),
            shell_mode: crate::config::ShellModeConfig::Auto,
            new_terminal_cwd: NewTerminalCwdConfig::Follow,
            pane_scrollback_limit_bytes: crate::config::DEFAULT_SCROLLBACK_LIMIT_BYTES,
            sound: SoundConfig {
                enabled: false,
                ..SoundConfig::default()
            },
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
            installed_plugins: std::collections::HashMap::new(),
            plugin_panes: std::collections::HashMap::new(),
            popup_pane: None,
            plugin_command_logs: Vec::new(),
            next_plugin_command_log_id: 1,
            plugin_commands_in_flight: 0,
            host_terminal_theme: TerminalTheme::default(),
            host_cell_size: crate::kitty_graphics::HostCellSize::default(),
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
            if let Some(toast) = &self.toast {
                assert!(
                    toast.target.is_none(),
                    "empty app state must not keep pane-targeted toast"
                );
            }
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

// Restored fork items (v0.9.3 sync): re-home next to their kin later.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspaceSort {
    #[default]
    Manual,
    Priority,
}

/// `size` if it is big enough to remember, or `None`. A zero or corrupt size
/// from a session file falls below the floor too.
pub(crate) fn rememberable_client_size(size: (u16, u16)) -> Option<(u16, u16)> {
    let (min_cols, min_rows) = REMEMBERED_CLIENT_SIZE_FLOOR;
    (size.0 >= min_cols && size.1 >= min_rows).then_some(size)
}

impl AppState {
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

    /// The terminal backing a pane, wherever that pane lives. Todos are stored
    /// on `TerminalState`, so every todo surface resolves through here.
    pub(crate) fn pane_terminal(&self, pane_id: PaneId) -> Option<&crate::terminal::TerminalState> {
        let pane = self
            .workspaces
            .iter()
            .find_map(|workspace| workspace.pane_state(pane_id))?;
        self.terminals.get(&pane.attached_terminal_id)
    }

    /// Post a herdr toast: shows it as the transient toast (behavior
    /// unchanged) and appends it to the notification log. All production
    /// toast sites go through here so the log stays complete.
    pub(crate) fn post_notification(&mut self, toast: ToastNotification) {
        self.notification_log.post(&toast, now_unix());
        self.toast = Some(toast);
    }
}

// Restored fork items (v0.9.3 sync): re-home next to their kin later.
/// The smallest client size, (cols, rows), remembered as the no-client size.
/// A quick look from a phone or a tiny split is used while it is attached,
/// but must not leave every pane that small once it detaches.
pub(crate) const REMEMBERED_CLIENT_SIZE_FLOOR: (u16, u16) = (80, 24);

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    #[test]
    fn new_pane_size_uses_headless_size_before_first_view() {
        let mut state = AppState::test_new();
        state.headless_size = (132, 41);
        state.pane_scrollbars = false;

        assert_eq!(
            state.new_pane_size(crate::ui::NewPanePlacement::Alone),
            (41, 132)
        );
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
        for name in crate::config::THEME_NAMES {
            assert!(
                Palette::from_name(name).is_some(),
                "theme should resolve: {name}"
            );
        }
    }

    #[test]
    fn built_in_active_rows_remain_visible_with_matching_terminal_backgrounds() {
        for name in crate::config::THEME_NAMES
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
        for name in crate::config::THEME_NAMES
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
        for name in crate::config::THEME_NAMES {
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
}
