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
