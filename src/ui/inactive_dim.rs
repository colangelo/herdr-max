//! The lighter, configurable dim for unfocused panes (fork issue 168).
//!
//! `[ui] inactive_pane_dim = N` moves the text colour of an unfocused pane's
//! cells N percent of the way toward the colour behind them. It recolours the
//! cells instead of setting the SGR faint attribute, whose strength is the
//! host terminal's, so the amount is predictable and stays apart from the
//! stronger faint that prefix and navigate mode add on top.
//!
//! This runs per cell of every unfocused pane on every render, so it takes
//! plain integers, allocates nothing and caches the last colour pair (runs of
//! identically styled cells are the common case).

use ratatui::style::{Color, Modifier};

use crate::terminal_theme::TerminalTheme;

type Rgb = (u8, u8, u8);

/// What a pane render needs to fade its cells: the percent, the colours that
/// stand in for "default" and the host palette for indexed colours.
pub(crate) struct InactiveDim<'a> {
    percent: u32,
    /// The colour behind a cell that kept the terminal's default background:
    /// the pane's tinted background, else the host terminal's.
    base: Rgb,
    /// The text colour of a cell that kept the terminal's default foreground.
    default_fg: Rgb,
    theme: &'a TerminalTheme,
    last: Option<((Color, Color), Color)>,
}

impl<'a> InactiveDim<'a> {
    /// `None` when there is nothing to do: a percent of 0, or no background to
    /// fade toward (the pane has none of its own, the host never reported one
    /// and the palette's panel background is not RGB).
    pub(crate) fn new(
        percent: u32,
        pane_bg: Option<Color>,
        panel_bg: Color,
        text: Color,
        theme: &'a TerminalTheme,
    ) -> Option<Self> {
        if percent == 0 {
            return None;
        }
        let base = pane_bg
            .and_then(|color| resolve(color, theme))
            .or_else(|| theme.background.map(|bg| (bg.r, bg.g, bg.b)))
            .or_else(|| resolve(panel_bg, theme))?;
        let default_fg = theme
            .foreground
            .map(|fg| (fg.r, fg.g, fg.b))
            .or_else(|| resolve(text, theme))?;
        Some(Self {
            percent: percent.min(100),
            base,
            default_fg,
            theme,
            last: None,
        })
    }

    /// The faded foreground for a cell, or `None` to leave it alone. Reversed
    /// cells swap which colour is the text, so they are left as they are.
    pub(crate) fn faded_fg(&mut self, fg: Color, bg: Color, modifier: Modifier) -> Option<Color> {
        if modifier.contains(Modifier::REVERSED) {
            return None;
        }
        if let Some((key, faded)) = self.last {
            if key == (fg, bg) {
                return Some(faded);
            }
        }
        let text = match fg {
            Color::Reset => self.default_fg,
            other => resolve(other, self.theme)?,
        };
        let behind = match bg {
            Color::Reset => self.base,
            other => resolve(other, self.theme).unwrap_or(self.base),
        };
        let faded = Color::Rgb(
            mix(text.0, behind.0, self.percent),
            mix(text.1, behind.1, self.percent),
            mix(text.2, behind.2, self.percent),
        );
        self.last = Some(((fg, bg), faded));
        Some(faded)
    }
}

/// `from` moved `percent` of the way to `to`. Integer math truncates toward
/// `from`, the same as the sidebar fog.
fn mix(from: u8, to: u8, percent: u32) -> u8 {
    let (from, to) = (i32::from(from), i32::from(to));
    (from + (to - from) * percent as i32 / 100).clamp(0, 255) as u8
}

/// A colour as RGB: RGB itself, the host terminal's palette entry for named
/// and indexed colours when the host reported it, else the xterm default.
/// `Reset` has no colour of its own and is `None`.
fn resolve(color: Color, theme: &TerminalTheme) -> Option<Rgb> {
    let index = match color {
        Color::Reset => return None,
        Color::Rgb(r, g, b) => return Some((r, g, b)),
        Color::Indexed(index) => index,
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
    };
    Some(
        theme.palette[usize::from(index)]
            .map(|c| (c.r, c.g, c.b))
            .unwrap_or_else(|| xterm_default(index)),
    )
}

/// The xterm 256-colour defaults, for hosts that did not report a palette.
fn xterm_default(index: u8) -> Rgb {
    const ANSI: [Rgb; 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match index {
        0..=15 => ANSI[usize::from(index)],
        16..=231 => {
            let n = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        232..=255 => {
            let v = 8 + (index - 232) * 10;
            (v, v, v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_theme::RgbColor;

    fn theme() -> TerminalTheme {
        let mut palette = [None; 256];
        palette[1] = Some(RgbColor {
            r: 200,
            g: 100,
            b: 0,
        });
        TerminalTheme {
            foreground: Some(RgbColor {
                r: 0xcd,
                g: 0xd6,
                b: 0xf4,
            }),
            background: Some(RgbColor {
                r: 0x0a,
                g: 0x0a,
                b: 0x0a,
            }),
            palette,
        }
    }

    fn dim(percent: u32, theme: &TerminalTheme) -> InactiveDim<'_> {
        InactiveDim::new(percent, None, Color::Reset, Color::Reset, theme).expect("a base")
    }

    #[test]
    fn default_text_fades_toward_the_host_background() {
        let theme = theme();
        let mut d = dim(20, &theme);
        // #cdd6f4 on #0a0a0a, 20%: 205 - 195 * 20 / 100 = 166 and so on.
        assert_eq!(
            d.faded_fg(Color::Reset, Color::Reset, Modifier::empty()),
            Some(Color::Rgb(0xa6, 0xae, 0xc6))
        );
    }

    #[test]
    fn the_pane_background_is_the_base_when_set() {
        let theme = theme();
        let mut d = InactiveDim::new(
            50,
            Some(Color::Rgb(0x30, 0x30, 0x30)),
            Color::Reset,
            Color::Reset,
            &theme,
        )
        .expect("a base");
        // #cdd6f4 halfway to #303030.
        assert_eq!(
            d.faded_fg(Color::Reset, Color::Reset, Modifier::empty()),
            Some(Color::Rgb(127, 131, 146))
        );
    }

    #[test]
    fn a_cell_with_its_own_background_fades_toward_that_background() {
        let theme = theme();
        let mut d = dim(50, &theme);
        assert_eq!(
            d.faded_fg(
                Color::Rgb(200, 200, 200),
                Color::Rgb(100, 100, 100),
                Modifier::empty()
            ),
            Some(Color::Rgb(150, 150, 150))
        );
    }

    #[test]
    fn named_and_indexed_colours_use_the_host_palette_then_xterm() {
        let theme = theme();
        let mut d = dim(100, &theme);
        // 100% is the background itself, so read the source colour at 0%.
        let mut zero = dim(1, &theme);
        assert_eq!(
            zero.faded_fg(Color::Red, Color::Reset, Modifier::empty()),
            Some(Color::Rgb(199, 100, 0)),
            "palette[1] is the host's orange, 1% toward #0a0a0a"
        );
        assert_eq!(
            zero.faded_fg(Color::Indexed(196), Color::Reset, Modifier::empty()),
            Some(Color::Rgb(253, 0, 0)),
            "196 is xterm #ff0000, 1% toward #0a0a0a"
        );
        assert_eq!(
            d.faded_fg(Color::Indexed(244), Color::Reset, Modifier::empty()),
            Some(Color::Rgb(0x0a, 0x0a, 0x0a))
        );
    }

    #[test]
    fn zero_percent_and_unknown_backgrounds_do_nothing() {
        let theme = theme();
        assert!(InactiveDim::new(0, None, Color::Reset, Color::Reset, &theme).is_none());
        let bare = TerminalTheme::default();
        assert!(InactiveDim::new(20, None, Color::Reset, Color::Reset, &bare).is_none());
        // A palette that is RGB stands in for a host that never answered.
        let mut d = InactiveDim::new(
            50,
            None,
            Color::Rgb(0, 0, 0),
            Color::Rgb(200, 200, 200),
            &bare,
        )
        .expect("the panel background is the base");
        assert_eq!(
            d.faded_fg(Color::Reset, Color::Reset, Modifier::empty()),
            Some(Color::Rgb(100, 100, 100))
        );
    }

    #[test]
    fn reversed_cells_are_left_alone() {
        let theme = theme();
        let mut d = dim(50, &theme);
        assert_eq!(
            d.faded_fg(Color::Reset, Color::Reset, Modifier::REVERSED),
            None
        );
    }

    #[test]
    fn xterm_defaults_cover_the_cube_and_the_grey_ramp() {
        assert_eq!(xterm_default(16), (0, 0, 0));
        assert_eq!(xterm_default(196), (255, 0, 0));
        assert_eq!(xterm_default(231), (255, 255, 255));
        assert_eq!(xterm_default(232), (8, 8, 8));
        assert_eq!(xterm_default(255), (238, 238, 238));
    }
}
