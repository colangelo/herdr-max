use ratatui::style::Color;

use super::{status_color, status_icon, ClientShellConfig, Palette};
use crate::{
    api::schema::AgentStatus,
    config::{Config, StateSymbolsConfig, StatusIndicatorStyle},
};

/// Local appearance, resolved once at startup/reload rather than in row loops.
pub(super) struct StatePresentation {
    colors: [Option<Color>; 6],
    symbols: StateSymbolsConfig,
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
        }
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
}
