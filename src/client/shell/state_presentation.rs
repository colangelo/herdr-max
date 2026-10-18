use ratatui::style::Color;

use super::{status_color, status_icon, ClientShellConfig, ClientShellSnapshot, Palette};
use crate::{
    api::schema::AgentStatus,
    config::{BackgroundMarkConfig, Config, StateSymbolsConfig, StatusIndicatorStyle},
};

/// The braille cell for 1 to 8 background items, one more dot each; more than
/// eight stay on the full cell. The right column fills first, then the left,
/// each from the bottom of its middle dots up and then its bottom and top ends
/// (6 5 8 4, then 3 2 7 1), so a small count sits low in the cell.
const BRAILLE: [&str; 8] = ["⠠", "⠰", "⢰", "⢸", "⢼", "⢾", "⣾", "⣿"];

/// The braille mark's cycle in spinner ticks (the frames pulse's period), and
/// the ticks at its end where the cell goes blank: the count stays readable
/// and still blinks once per cycle.
const BRAILLE_CYCLE_TICKS: u8 = 16;
const BRAILLE_OFF_TICKS: u8 = 3;

/// What the server says an agent row has running in the background.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Background {
    /// Held Working by work it launched (an upstream `background_*` rule).
    pub(super) work: bool,
    /// Background items listed on the agent's screen; 0 for none.
    pub(super) count: u8,
}

impl From<bool> for Background {
    fn from(work: bool) -> Self {
        Self { work, count: 0 }
    }
}

impl Background {
    pub(super) fn from_snapshot(snapshot: &ClientShellSnapshot, pane_id: &str) -> Self {
        let facts = snapshot.resource_facts.as_ref();
        Self {
            work: facts
                .and_then(|facts| facts.background_activity.as_ref())
                .and_then(|map| map.get(pane_id))
                .copied()
                .unwrap_or(false),
            count: facts
                .and_then(|facts| facts.background_count.as_ref())
                .and_then(|map| map.get(pane_id))
                .copied()
                .unwrap_or(0),
        }
    }
}

/// Local appearance, resolved once at startup/reload rather than in row loops.
pub(super) struct StatePresentation {
    colors: [Option<Color>; 6],
    symbols: StateSymbolsConfig,
    background_mark: BackgroundMarkConfig,
}

impl StatePresentation {
    pub(super) fn from_config(config: &Config) -> Self {
        let colors = &config.ui.state_colors;
        let parse = |value: &Option<String>| value.as_deref().map(crate::config::parse_color);
        let mut symbols = config.ui.state_symbols.clone();
        for value in [
            &mut symbols.working,
            &mut symbols.idle,
            &mut symbols.done,
            &mut symbols.blocked,
            &mut symbols.unknown,
            &mut symbols.background,
            &mut symbols.background_alt,
        ] {
            if StateSymbolsConfig::valid(value).is_none() {
                *value = None;
            }
        }
        Self {
            colors: [
                parse(&colors.working),
                parse(&colors.idle),
                parse(&colors.done),
                parse(&colors.blocked),
                parse(&colors.unknown),
                parse(&colors.background),
            ],
            symbols,
            background_mark: config.ui.background_mark,
        }
    }

    /// Whether `background` is drawn as the braille mark on this row: the mark
    /// style is on and the row is idle or done with items running, or is
    /// parked Working on them. A working turn keeps its spinner; blocked and
    /// unknown rows keep their own glyph.
    fn braille(&self, status: AgentStatus, background: Background) -> bool {
        self.background_mark == BackgroundMarkConfig::Braille
            && background.count > 0
            && match status {
                AgentStatus::Idle | AgentStatus::Done => true,
                AgentStatus::Working => background.work,
                AgentStatus::Blocked | AgentStatus::Unknown => false,
            }
    }

    pub(super) fn background_mark(&self) -> BackgroundMarkConfig {
        self.background_mark
    }

    pub(super) fn icon(&self, status: AgentStatus, style: StatusIndicatorStyle) -> &str {
        let glyph = match status {
            AgentStatus::Working => &self.symbols.working,
            AgentStatus::Idle => &self.symbols.idle,
            AgentStatus::Done => &self.symbols.done,
            AgentStatus::Blocked => &self.symbols.blocked,
            AgentStatus::Unknown => &self.symbols.unknown,
        };
        glyph
            .as_deref()
            .unwrap_or_else(|| status_icon(status, style))
    }

    pub(super) fn agent_icon(
        &self,
        status: AgentStatus,
        background: impl Into<Background>,
        seq: u64,
        config: &ClientShellConfig,
    ) -> &str {
        // The classic braille snake one row down, on the cell's bottom three
        // rows (dots 2 3 7 and 5 6 8), so it sits low like the braille mark.
        const FRAMES: [&str; 10] = ["⠖", "⠲", "⢲", "⢰", "⣰", "⣠", "⣄", "⣆", "⡆", "⡖"];
        let background: Background = background.into();
        let frame = (config.status_spinner == crate::config::StatusSpinnerConfig::On)
            .then_some(config.spinner_frame.wrapping_add(seq as u8));
        if self.braille(status, background) {
            // A still spinner keeps the small dot; a running one blinks the count.
            return match frame {
                None => self.symbols.background.as_deref().unwrap_or("·"),
                Some(frame)
                    if frame % BRAILLE_CYCLE_TICKS < BRAILLE_CYCLE_TICKS - BRAILLE_OFF_TICKS =>
                {
                    BRAILLE[usize::from(background.count.min(8)) - 1]
                }
                Some(_) => " ",
            };
        }
        if status != AgentStatus::Working {
            return self.icon(status, config.status_indicators);
        }
        if background.work {
            return if frame.is_some_and(|frame| (frame / 8) % 2 == 1) {
                self.symbols.background_alt.as_deref().unwrap_or("◆")
            } else {
                self.symbols.background.as_deref().unwrap_or("■")
            };
        }
        frame
            .map(|frame| FRAMES[usize::from(frame) % FRAMES.len()])
            .unwrap_or_else(|| self.icon(status, config.status_indicators))
    }
    pub(super) fn agent_color(
        &self,
        status: AgentStatus,
        background: impl Into<Background>,
        palette: &Palette,
    ) -> Color {
        let background: Background = background.into();
        if (status == AgentStatus::Working && background.work) || self.braille(status, background) {
            self.colors[5].unwrap_or_else(|| self.color(status, palette))
        } else {
            self.color(status, palette)
        }
    }
    pub(super) fn color(&self, status: AgentStatus, palette: &Palette) -> Color {
        let index = match status {
            AgentStatus::Working => 0,
            AgentStatus::Idle => 1,
            AgentStatus::Done => 2,
            AgentStatus::Blocked => 3,
            AgentStatus::Unknown => 4,
        };
        self.colors[index].unwrap_or_else(|| status_color(status, palette))
    }
}

impl ClientShellConfig {
    pub(super) fn state_icon(&self, status: AgentStatus) -> &str {
        self.state_presentation.icon(status, self.status_indicators)
    }
    pub(super) fn state_color(&self, status: AgentStatus) -> Color {
        self.state_presentation.color(status, &self.palette)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_symbols_use_fork_defaults_and_ignore_multi_cell_overrides() {
        let mut config = Config::default();
        config.ui.status_indicators = StatusIndicatorStyle::Symbols;
        config.ui.state_symbols.working = Some("界".into());
        config.ui.state_symbols.blocked = Some("!".into());
        let shell = ClientShellConfig::from_config(&config);
        assert_eq!(shell.state_icon(AgentStatus::Done), "□");
        assert_eq!(shell.state_icon(AgentStatus::Idle), "✓");
        assert_eq!(shell.state_icon(AgentStatus::Working), "◐");
        assert_eq!(shell.state_icon(AgentStatus::Blocked), "!");
    }

    #[test]
    fn state_colors_and_symbols_apply_on_live_reload_independently_of_style() {
        let mut config = Config::default();
        let mut shell = ClientShellConfig::from_config(&config);
        assert_eq!(
            shell.state_color(AgentStatus::Working),
            shell.palette.yellow
        );
        config.ui.state_colors.working = Some("#123456".into());
        config.ui.state_symbols.working = Some("w".into());
        shell.apply_live_config(&config, &[], &[]);
        assert_eq!(
            shell.state_color(AgentStatus::Working),
            Color::Rgb(0x12, 0x34, 0x56)
        );
        assert_eq!(shell.state_icon(AgentStatus::Working), "w");
        assert_eq!(shell.state_color(AgentStatus::Blocked), shell.palette.red);
    }
    #[test]
    fn working_animation_and_background_pulse_match_fork_frames_and_static_fallback() {
        let mut config = ClientShellConfig::from_config(&Config::default());
        assert_eq!(
            config
                .state_presentation
                .agent_icon(AgentStatus::Working, false, 0, &config),
            "⠖"
        );
        assert_eq!(
            config
                .state_presentation
                .agent_icon(AgentStatus::Working, false, 1, &config),
            "⠲"
        );
        // The snake runs on the bottom three rows: no frame lights the top
        // row's dots 1 and 4.
        for seq in 0..10 {
            let frame =
                config
                    .state_presentation
                    .agent_icon(AgentStatus::Working, false, seq, &config);
            let dots = frame.chars().next().map_or(0, |c| u32::from(c) - 0x2800);
            assert_eq!(dots & 0x09, 0, "frame {seq} is {frame}");
        }
        assert_eq!(
            config
                .state_presentation
                .agent_icon(AgentStatus::Working, true, 0, &config),
            "■"
        );
        config.spinner_frame = 8;
        assert_eq!(
            config
                .state_presentation
                .agent_icon(AgentStatus::Working, true, 0, &config),
            "◆"
        );
        config.status_spinner = crate::config::StatusSpinnerConfig::Off;
        assert_eq!(
            config
                .state_presentation
                .agent_icon(AgentStatus::Working, true, 0, &config),
            "■"
        );
        assert_eq!(
            config
                .state_presentation
                .agent_icon(AgentStatus::Working, false, 0, &config),
            "●"
        );
    }

    fn braille_config() -> ClientShellConfig {
        let mut config = Config::default();
        config.ui.background_mark = crate::config::BackgroundMarkConfig::Braille;
        ClientShellConfig::from_config(&config)
    }

    fn mark(status: AgentStatus, count: u8, frame: u8) -> String {
        let mut config = braille_config();
        config.spinner_frame = frame;
        config
            .state_presentation
            .agent_icon(status, Background { work: false, count }, 0, &config)
            .to_owned()
    }

    #[test]
    fn braille_mark_shows_the_count_and_blinks_blank_at_the_cycle_end() {
        // Ticks 0..13 of each 16: the braille cell. Ticks 13 to 15: blank.
        for frame in [0, 1, 8, 12, 16, 28] {
            assert_eq!(mark(AgentStatus::Idle, 3, frame), "⢰", "frame {frame}");
        }
        for frame in [13, 15, 29, 31, 253, 255] {
            assert_eq!(mark(AgentStatus::Idle, 3, frame), " ", "frame {frame}");
        }
        assert_eq!(mark(AgentStatus::Done, 1, 8), "⠠");
        assert_eq!(mark(AgentStatus::Idle, 2, 8), "⠰");
        assert_eq!(mark(AgentStatus::Idle, 4, 8), "⢸");
        assert_eq!(mark(AgentStatus::Idle, 5, 8), "⢼");
        assert_eq!(mark(AgentStatus::Idle, 6, 8), "⢾");
        assert_eq!(mark(AgentStatus::Idle, 7, 8), "⣾");
        assert_eq!(mark(AgentStatus::Idle, 8, 8), "⣿");
        assert_eq!(mark(AgentStatus::Idle, 200, 8), "⣿");
    }

    #[test]
    fn braille_table_has_eight_one_cell_glyphs() {
        for glyph in BRAILLE {
            assert_eq!(unicode_width::UnicodeWidthStr::width(glyph), 1, "{glyph}");
        }
        assert_eq!(BRAILLE.len(), 8);
    }

    #[test]
    fn frames_mode_keeps_an_idle_row_with_background_work_plain() {
        let config = ClientShellConfig::from_config(&Config::default());
        assert_eq!(
            config.state_presentation.agent_icon(
                AgentStatus::Idle,
                Background {
                    work: false,
                    count: 3
                },
                0,
                &config
            ),
            config.state_icon(AgentStatus::Idle)
        );
    }

    #[test]
    fn braille_leaves_blocked_and_a_working_turn_alone() {
        let config = braille_config();
        assert_eq!(
            mark(AgentStatus::Blocked, 3, 8),
            config.state_icon(AgentStatus::Blocked)
        );
        // Working because of the agent's own turn keeps the spinner.
        assert_eq!(mark(AgentStatus::Working, 3, 0), "⠖");
    }

    #[test]
    fn braille_with_the_spinner_off_shows_only_the_dot() {
        let mut config = braille_config();
        config.status_spinner = crate::config::StatusSpinnerConfig::Off;
        let background = Background {
            work: false,
            count: 3,
        };
        assert_eq!(
            config
                .state_presentation
                .agent_icon(AgentStatus::Idle, background, 8, &config),
            "·"
        );
    }

    #[test]
    fn braille_rows_take_the_background_colour() {
        let mut cfg = Config::default();
        cfg.ui.background_mark = crate::config::BackgroundMarkConfig::Braille;
        cfg.ui.state_colors.background = Some("#112233".into());
        let config = ClientShellConfig::from_config(&cfg);
        let background = Background {
            work: false,
            count: 2,
        };
        assert_eq!(
            config
                .state_presentation
                .agent_color(AgentStatus::Idle, background, &config.palette),
            Color::Rgb(0x11, 0x22, 0x33)
        );
        assert_eq!(
            config.state_presentation.agent_color(
                AgentStatus::Idle,
                Background::default(),
                &config.palette
            ),
            config.state_color(AgentStatus::Idle)
        );
    }
}
