use super::render::display_width;
use super::*;

pub(super) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    palette: &Palette,
) -> Option<Rect> {
    let sync = snapshot
        .resource_facts
        .as_ref()?
        .tab_sync
        .as_ref()?
        .get(snapshot.focused_tab_id.as_ref()?)?;
    if area.is_empty() {
        return None;
    }
    let text = if sync.ending {
        " SYNC ending… ".to_owned()
    } else {
        let count = sync.members.len();
        format!(
            " SYNC {count} {} ",
            if count == 1 { "pane" } else { "panes" }
        )
    };
    let rect = Rect::new(
        area.x,
        area.bottom() - 1,
        display_width(&text).min(area.width),
        1,
    );
    buffer.set_stringn(
        rect.x,
        rect.y,
        text,
        usize::from(rect.width),
        // SYNC keeps its own yellow; its text is the dark chip text of every mode chip.
        Style::default()
            .fg(panel_contrast_fg(palette))
            .bg(crate::app::state::SYNC_YELLOW)
            .add_modifier(Modifier::BOLD),
    );
    Some(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sync_chip_counts_members_and_grace_with_original_yellow() {
        let mut snapshot = crate::client::shell::tests::snapshot();
        let tab = snapshot.focused_tab_id.clone().unwrap();
        snapshot.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
            tab_sync: Some(
                [(
                    tab.clone(),
                    crate::protocol::ClientTabSync {
                        members: vec!["w1:p1".into()],
                        ending: false,
                    },
                )]
                .into(),
            ),
            ..Default::default()
        });
        let palette = ClientShellConfig::from_config(&Config::default()).palette;
        let area = Rect::new(0, 0, 30, 3);
        let mut buffer = Buffer::empty(area);
        let rect = paint(&mut buffer, area, &snapshot, &palette).unwrap();
        let row: String = (rect.x..rect.right())
            .map(|x| buffer[(x, rect.y)].symbol())
            .collect();
        assert_eq!(row, " SYNC 1 pane ");
        assert_eq!(
            buffer[(1, 2)].bg,
            ratatui::style::Color::Rgb(0xff, 0xd6, 0x0a)
        );
        snapshot
            .resource_facts
            .as_mut()
            .unwrap()
            .tab_sync
            .as_mut()
            .unwrap()
            .get_mut(&tab)
            .unwrap()
            .ending = true;
        let rect = paint(&mut buffer, area, &snapshot, &palette).unwrap();
        let row: String = (rect.x..rect.right())
            .map(|x| buffer[(x, rect.y)].symbol())
            .collect();
        assert_eq!(row, " SYNC ending… ");
        snapshot.resource_facts = None;
        assert!(paint(&mut buffer, area, &snapshot, &palette).is_none());
    }

    #[test]
    fn sync_chip_keeps_its_yellow_with_dark_bold_text_on_every_palette() {
        // Fork issue 174: SYNC keeps its own colour, and its text is the dark chip text the other
        // mode chips use, also on the terminal palette whose panel background is the terminal's.
        let mut snapshot = crate::client::shell::tests::snapshot();
        let tab = snapshot.focused_tab_id.clone().unwrap();
        snapshot.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
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
        for palette in [
            ClientShellConfig::from_config(&Config::default()).palette,
            crate::app::state::Palette::terminal(),
        ] {
            let area = Rect::new(0, 0, 30, 3);
            let mut buffer = Buffer::empty(area);
            let rect = paint(&mut buffer, area, &snapshot, &palette).unwrap();
            let cell = &buffer[(rect.x + 1, rect.y)];
            assert_eq!(cell.bg, crate::app::state::SYNC_YELLOW);
            assert_eq!(cell.fg, panel_contrast_fg(&palette));
            assert_ne!(cell.fg, ratatui::style::Color::Reset);
            assert!(cell.modifier.contains(Modifier::BOLD));
        }
    }
}
