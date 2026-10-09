//! The look ac approved for herdr's own bars (unify-tui-look, fork issue 174): a mode keeps its own
//! colour for its chip and its keys, labels are dim, and everything that is not a mode bar uses the
//! accent. https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/174
use super::*;

fn mode_bar(mode: ClientShellMode) -> (Buffer, Palette) {
    let config = ClientShellConfig::from_config(&Config::default());
    let mut buffer = Buffer::empty(Rect::new(0, 0, 160, 3));
    render::render_mode_bar(
        &mut buffer,
        Rect::new(0, 0, 160, 3),
        mode,
        None,
        None,
        false,
        &config.keybinds,
        &config.palette,
    )
    .unwrap();
    (buffer, config.palette)
}

fn at(buffer: &Buffer, needle: &str) -> u16 {
    let row: String = (0..buffer.area.width)
        .map(|x| buffer[(x, 2)].symbol())
        .collect();
    let byte = row
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in {row:?}"));
    row[..byte].chars().count() as u16
}

#[test]
fn each_mode_bar_paints_its_chip_and_its_keys_in_the_modes_colour() {
    for (mode, chip, key, colour) in [
        (ClientShellMode::Navigate, " NAVIGATE ", "esc", "accent"),
        (ClientShellMode::Prefix, " NAVIGATE ", "esc", "accent"),
        (ClientShellMode::Scroll, " SCROLL ", "q/esc", "accent"),
        (ClientShellMode::Resize, " RESIZE ", "h/l", "mauve"),
    ] {
        let (buffer, p) = mode_bar(mode);
        let colour = if colour == "mauve" { p.mauve } else { p.accent };
        let chip_x = at(&buffer, chip);
        assert_eq!(buffer[(chip_x, 2)].bg, colour, "{chip} chip");
        assert!(buffer[(chip_x, 2)].modifier.contains(Modifier::BOLD));
        let key_x = at(&buffer, key);
        assert_eq!(buffer[(key_x, 2)].fg, colour, "{chip} key {key}");
        assert!(buffer[(key_x, 2)].modifier.contains(Modifier::BOLD));
    }
}

#[test]
fn mode_bar_labels_are_dim() {
    let (buffer, p) = mode_bar(ClientShellMode::Resize);
    let x = at(&buffer, "width");
    assert_eq!(buffer[(x, 2)].fg, p.overlay0);
    assert!(!buffer[(x, 2)].modifier.contains(Modifier::BOLD));
}

#[test]
fn the_announcement_footer_is_a_key_bar() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut endpoint_snapshot = snapshot();
    endpoint_snapshot.product_announcement =
        Some(crate::protocol::ClientShellProductAnnouncement {
            version: "0.8.2".into(),
            id: "client-shell".into(),
            title: "A client-owned announcement".into(),
            body: "one line".into(),
            preview: false,
        });
    state.set_snapshot(Box::new(endpoint_snapshot));
    state.set_pane_surface(surface_with_popup());
    let frame = state.compose(106, 30).expect("announcement frame");
    let area = Rect::new(0, 0, 106, 30);
    let (x, y) = cell_symbol_position(&frame, area, "wheel ↑↓ scroll  esc/enter close");
    let p = &state.config.palette;
    let cell = |x: u16| &frame.cells[usize::from(y) * 106 + usize::from(x)];
    assert_eq!(cell(x).fg, crate::protocol::color_to_u32(p.accent), "key");
    assert_eq!(
        cell(x + 9).fg,
        crate::protocol::color_to_u32(p.overlay0),
        "label"
    );
}

// -- captures for review (run with `--run-ignored only --no-capture -E 'test(capture_)'`) ----------

/// Print `rows` of `frame` twice: as 24-bit ANSI to look at in a terminal, and as a paint map with one
/// code per cell (`A` accent fill, `P` red fill, `M` mauve fill, `Y` sync yellow fill, `U` surface0
/// fill, `k` accent text, `p` red text, `m` mauve text, `d` dim, `s` rule, `.` other text).
pub(super) fn capture(name: &str, frame: &FrameData, rows: std::ops::Range<u16>, p: &Palette) {
    let c = crate::protocol::color_to_u32;
    let ansi = |packed: u32, ground: u8| -> String {
        if packed >> 24 == 0x02 {
            format!(
                "\x1b[{ground};2;{};{};{}m",
                (packed >> 16) & 0xff,
                (packed >> 8) & 0xff,
                packed & 0xff
            )
        } else {
            String::new()
        }
    };
    println!("=== {name} ===");
    let mut map = Vec::new();
    for y in rows {
        let mut line = String::new();
        let mut paint = String::new();
        for x in 0..frame.width {
            let cell = &frame.cells[usize::from(y) * usize::from(frame.width) + usize::from(x)];
            line.push_str(&ansi(cell.fg, 38));
            line.push_str(&ansi(cell.bg, 48));
            line.push_str(&cell.symbol);
            line.push_str("\x1b[0m");
            let code = if cell.bg == c(p.accent) {
                'A'
            } else if cell.bg == c(p.red) {
                'P'
            } else if cell.bg == c(p.mauve) {
                'M'
            } else if cell.bg == c(crate::app::state::SYNC_YELLOW) {
                'Y'
            } else if cell.bg == c(p.surface0) && p.surface0 != p.panel_bg {
                'U'
            } else if cell.symbol.trim().is_empty() {
                ' '
            } else if cell.fg == c(p.accent) {
                'k'
            } else if cell.fg == c(p.red) {
                'p'
            } else if cell.fg == c(p.mauve) {
                'm'
            } else if cell.fg == c(p.overlay0) {
                'd'
            } else if cell.fg == c(p.surface1) {
                's'
            } else {
                '.'
            };
            paint.push(code);
        }
        println!("{line}");
        map.push(paint);
    }
    println!("--- paint map ---");
    for row in map {
        println!("{}", row.trim_end());
    }
}

fn composed_bar(mode: ClientShellMode, explicit_labels: bool, sync: bool) -> (FrameData, Palette) {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snap = snapshot();
    if sync {
        let tab = snap.focused_tab_id.clone().unwrap();
        snap.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
            tab_sync: Some(
                [(
                    tab,
                    crate::protocol::ClientTabSync {
                        members: vec!["w1:p1".into(), "w1:p2".into()],
                        ending: false,
                    },
                )]
                .into(),
            ),
            ..Default::default()
        });
    }
    state.set_snapshot(Box::new(snap));
    state.set_endpoint_server_version(&ClientEndpointId::Local, Some("0.9.3-ac-beta".into()));
    let layout = state.layout(140, 12);
    let mut surface = surface();
    let area = Rect::new(0, 0, layout.pane_surface.width, layout.pane_surface.height);
    surface.frame = FrameData::from_ratatui_buffer_with_hyperlinks(&Buffer::empty(area), None, &[]);
    surface.panes[0].rect.width = area.width;
    surface.panes[0].rect.height = area.height;
    surface.panes[0].inner_rect = surface.panes[0].rect;
    state.set_pane_surface(surface);
    state.mode = mode;
    if explicit_labels {
        state.arm_pane_labels(true, std::time::Instant::now());
    }
    let frame = state.compose(140, 12).unwrap();
    (frame, state.config.palette)
}

#[test]
#[ignore = "capture for review, not a check"]
fn capture_mode_bars() {
    let (frame, p) = composed_bar(ClientShellMode::Navigate, true, false);
    capture("PANES bar (display panes)", &frame, 11..12, &p);
    let (frame, p) = composed_bar(ClientShellMode::Resize, false, false);
    capture("RESIZE bar", &frame, 11..12, &p);
    let (frame, p) = composed_bar(ClientShellMode::Terminal, false, true);
    capture("SYNC chip", &frame, 11..12, &p);
    let (frame, p) = composed_bar(ClientShellMode::Navigate, false, false);
    capture("NAVIGATE bar", &frame, 11..12, &p);
}
