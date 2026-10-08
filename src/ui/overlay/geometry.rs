//! One definition of where an anchored panel sits.
//!
//! The notification center and the pane todo panel each used to carry ~60 lines
//! performing the identical sequence — pick an anchor, clamp a measured width,
//! count rows, reserve a footer, place x and y against the screen, then derive
//! the inner rect, the list rect and the footer row from the result. They
//! differed in what they measured and where they anchored, and in nothing else.

use ratatui::layout::Rect;

use crate::ui::widgets::footer_split;

/// The widest a centred list dialog grows for its content: the navigator, the
/// move picker and the todo board. Past this a row is read across too much
/// empty screen, and the footer's buttons end up far from the rows they act on.
pub(crate) const LIST_DIALOG_MAX_WIDTH: u16 = 120;

/// The fewest rows a detail box is drawn in: its two borders and one row of
/// text. Less than that is a lone rule, not a box.
pub(crate) const DETAIL_MIN_ROWS: u16 = 3;

/// Where a panel's detail box sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetailPlacement {
    /// Between the list and the footer.
    BelowList,
    /// Between the header block and the list: title, search, box, list.
    // Not the todo layout any more (ac picked `BelowTitle`, fork issue 142);
    // the alternative stays selectable and tested.
    #[allow(dead_code)]
    AboveList,
    /// Directly under the header's first row: title, box, search, list. A
    /// panel with no header takes [`DetailPlacement::AboveList`].
    BelowTitle,
}

/// Where a panel's top edge comes from. Horizontal placement follows from
/// it: an anchored panel right-aligns to its anchor, and a centred one is
/// centred on the screen in both directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerticalAnchor {
    /// Hangs under the anchor's bottom edge — a dropdown from a tab bar.
    Below,
    /// Starts one row inside the anchor's top edge, so it drops out of a
    /// border indicator rather than covering it.
    InsideTop,
    /// Opens above this rect's top edge, so the thing that toggles the panel
    /// stays visible underneath it (the global-launcher idiom). An empty rect
    /// means there is nothing to clear, and the panel sits flush at the bottom.
    Above(Rect),
    /// Centred on the screen, for a dialog that belongs to the session rather
    /// than to anything on it. The anchor is ignored.
    Centered,
}

/// What a caller knows about its own panel. Everything here is either constant
/// per panel or already measured at the call site.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AnchoredPanelSpec {
    /// The panel right-aligns to this rect's right edge.
    pub anchor: Rect,
    /// Everything the panel may be placed within.
    pub screen: Rect,
    /// The panel's desired outer width: what the caller measured its own rows
    /// to need, its own chrome included. Measuring is the one genuinely
    /// per-panel thing — one panel counts a title plus a context plus an age
    /// column, the other a label plus a link chip — so it stays with the
    /// caller and everything downstream of it does not.
    pub content_width: u16,
    /// Inclusive clamp on the resolved width, before the screen has its say.
    pub width_bounds: (u16, u16),
    /// Content rows the panel would like. Always at least one, so an empty
    /// panel is still a panel.
    pub rows: u16,
    /// Cap on `rows`.
    pub max_rows: u16,
    /// Rows reserved below the list, normally [`crate::ui::FOOTER_ROWS`]. Zero
    /// for a panel with no footer to show.
    pub footer_rows: u16,
    /// Rows reserved between the list and the footer for a detail box, its
    /// borders included. Zero for a panel that shows no detail, which is
    /// every panel whose rows have nothing more to say — the box appears only
    /// when it has something to hold.
    pub detail_rows: u16,
    /// Where those rows go.
    pub detail_placement: DetailPlacement,
    /// Rows above the list for the panel's own header — a title, a search
    /// row, the blank row under them. Zero for a panel whose list starts at
    /// its top border.
    pub header_rows: u16,
    /// Where the panel's top edge comes from.
    pub vertical: VerticalAnchor,
}

/// Everything an anchored panel is drawn and hit-tested against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PanelGeometry {
    /// The panel including its border.
    pub outer: Rect,
    /// The panel minus its border: list and footer together.
    pub inner: Rect,
    /// The list rect, stopping short of the footer block.
    pub list: Rect,
    /// The footer's button row, absent when there is no room for the whole
    /// footer block or nothing to put in it.
    pub footer_row: Option<Rect>,
    /// The detail box between list and footer, borders included, absent when
    /// the panel asked for none or fewer than [`DETAIL_MIN_ROWS`] were free.
    pub detail: Option<Rect>,
    /// The header rows above the list; empty when the panel asked for none.
    /// A detail box placed under the title sits inside it, between its first
    /// row and the rest: read the rows through [`PanelGeometry::header_row`].
    pub header: Rect,
    detail_in_header: bool,
}

impl PanelGeometry {
    /// Header row `index` (0 is the title), stepping over a detail box that
    /// sits under the title.
    pub(crate) fn header_row(&self, index: u16) -> Rect {
        let skipped = match self.detail {
            Some(detail) if self.detail_in_header && index >= 1 => detail.height,
            _ => 0,
        };
        Rect::new(
            self.header.x,
            self.header.y + index + skipped,
            self.header.width,
            1,
        )
    }
}

impl AnchoredPanelSpec {
    /// The width `resolve` will settle on, for a caller that has to lay text
    /// out inside the panel before it can say how many rows that text needs.
    pub(crate) fn resolved_width(&self) -> u16 {
        let (min_width, max_width) = self.width_bounds;
        self.content_width
            .clamp(min_width, max_width)
            .min(self.screen.width.max(1))
    }

    /// Resolve the panel's placement, or `None` when there is no screen to
    /// place it on — render and hit-test go quiet together.
    pub(crate) fn resolve(&self) -> Option<PanelGeometry> {
        let screen = self.screen;
        if screen.width == 0 || screen.height == 0 {
            return None;
        }

        let width = self.resolved_width();
        let rows = self.rows.max(1).min(self.max_rows);
        let height = (self.header_rows + rows + 2 + self.footer_rows + self.detail_rows)
            .min(screen.height.max(1));

        let x = if self.vertical == VerticalAnchor::Centered {
            screen.x + screen.width.saturating_sub(width) / 2
        } else {
            let right = self.anchor.x.saturating_add(self.anchor.width);
            right.saturating_sub(width).max(screen.x)
        };

        // The lowest top edge that still leaves room for the whole panel.
        let bottom_y = screen.y + screen.height.saturating_sub(height);
        let top = match self.vertical {
            VerticalAnchor::Below => self.anchor.y.saturating_add(self.anchor.height),
            VerticalAnchor::InsideTop => self.anchor.y.saturating_add(1),
            VerticalAnchor::Above(over) if over.width > 0 => over.y.saturating_sub(height),
            VerticalAnchor::Above(_) => bottom_y,
            VerticalAnchor::Centered => screen.y + screen.height.saturating_sub(height) / 2,
        };
        let y = top.min(bottom_y).max(screen.y);

        let outer = Rect::new(x, y, width, height);
        let inner = Rect::new(
            outer.x + 1,
            outer.y + 1,
            outer.width.saturating_sub(2),
            outer.height.saturating_sub(2),
        );
        // A footer that cannot fit its whole block, blank row included, is not
        // drawn — the buttons would otherwise sit flush against the last row.
        let has_footer =
            self.footer_rows > 0 && inner.width > 0 && inner.height >= self.footer_rows;
        let (list, _) = footer_split(inner, has_footer);
        let footer_row =
            has_footer.then(|| Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1));

        // The header is carved off the top of the list, so what is left is
        // exactly the rows the list scrolls in.
        let header_rows = self.header_rows.min(list.height);
        let header = Rect::new(list.x, list.y, list.width, header_rows);
        let list = Rect::new(
            list.x,
            list.y + header_rows,
            list.width,
            list.height - header_rows,
        );

        // The detail box is carved off the list. It yields to the list rather
        // than the other way round: a panel squeezed to nothing shows its
        // todos, not a detail of one of them, and a box with no room for a row
        // of text is not drawn at all.
        let detail_rows = self.detail_rows.min(list.height.saturating_sub(1));
        let mut header = header;
        let mut detail_in_header = false;
        let (list, detail) = if detail_rows >= DETAIL_MIN_ROWS {
            let placement = match self.detail_placement {
                DetailPlacement::BelowTitle if header.height == 0 => DetailPlacement::AboveList,
                placement => placement,
            };
            let shrunk = list.height - detail_rows;
            match placement {
                DetailPlacement::BelowList => (
                    Rect::new(list.x, list.y, list.width, shrunk),
                    Some(Rect::new(list.x, list.y + shrunk, list.width, detail_rows)),
                ),
                DetailPlacement::AboveList => (
                    Rect::new(list.x, list.y + detail_rows, list.width, shrunk),
                    Some(Rect::new(list.x, list.y, list.width, detail_rows)),
                ),
                DetailPlacement::BelowTitle => {
                    detail_in_header = true;
                    let detail = Rect::new(header.x, header.y + 1, header.width, detail_rows);
                    header.height += detail_rows;
                    (
                        Rect::new(list.x, list.y + detail_rows, list.width, shrunk),
                        Some(detail),
                    )
                }
            }
        } else {
            (list, None)
        };

        Some(PanelGeometry {
            outer,
            inner,
            list,
            footer_row,
            detail,
            header,
            detail_in_header,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 25,
    };

    fn spec(vertical: VerticalAnchor) -> AnchoredPanelSpec {
        AnchoredPanelSpec {
            anchor: Rect::new(0, 0, 80, 1),
            screen: SCREEN,
            content_width: 40,
            width_bounds: (30, 60),
            rows: 3,
            max_rows: 12,
            footer_rows: crate::ui::FOOTER_ROWS,
            detail_rows: 0,
            detail_placement: DetailPlacement::BelowList,
            header_rows: 0,
            vertical,
        }
    }

    #[test]
    fn centred_panel_is_measured_bounded_and_centred() {
        let screen = Rect::new(0, 0, 310, 56);
        let centred = |content_width: u16, screen: Rect| AnchoredPanelSpec {
            anchor: Rect::default(),
            screen,
            content_width,
            width_bounds: (64, LIST_DIALOG_MAX_WIDTH),
            rows: 10,
            max_rows: 40,
            footer_rows: crate::ui::FOOTER_ROWS,
            detail_rows: 0,
            detail_placement: DetailPlacement::BelowList,
            header_rows: 3,
            vertical: VerticalAnchor::Centered,
        };

        let small = centred(40, screen).resolve().expect("resolves");
        assert_eq!(small.outer.width, 64, "the floor");
        assert_eq!(small.outer.x, (310 - 64) / 2, "centred across");
        assert_eq!(small.outer.y, (56 - small.outer.height) / 2, "centred down");
        assert_eq!(small.outer.height, 3 + 10 + 2 + crate::ui::FOOTER_ROWS);

        let measured = centred(90, screen).resolve().expect("resolves");
        assert_eq!(measured.outer.width, 90, "its own content");

        let wide = centred(272, screen).resolve().expect("resolves");
        assert_eq!(wide.outer.width, LIST_DIALOG_MAX_WIDTH, "the shared cap");
        assert_eq!(wide.outer.x, (310 - 120) / 2);

        let narrow_screen = Rect::new(0, 0, 50, 20);
        let cramped = centred(90, narrow_screen).resolve().expect("resolves");
        assert_eq!(cramped.outer.width, 50, "never wider than the screen");
        assert_eq!(cramped.outer.x, 0);

        // The header sits above the list and the list starts under it.
        assert_eq!(small.header.height, 3);
        assert_eq!(small.header.y, small.inner.y);
        assert_eq!(small.list.y, small.inner.y + 3);
        assert_eq!(small.list.height, 10);
    }

    /// Fork issue 142: the same box can sit under the list, above it (title,
    /// search, box, list) or under the title (title, box, search, list).
    #[test]
    fn the_detail_box_goes_where_the_spec_puts_it() {
        let resolve = |detail_placement| {
            AnchoredPanelSpec {
                detail_rows: 5,
                detail_placement,
                header_rows: 3,
                ..AnchoredPanelSpec {
                    anchor: SCREEN,
                    screen: SCREEN,
                    content_width: 40,
                    width_bounds: (20, 60),
                    rows: 10,
                    max_rows: 10,
                    footer_rows: crate::ui::FOOTER_ROWS,
                    detail_rows: 0,
                    detail_placement: DetailPlacement::BelowList,
                    header_rows: 0,
                    vertical: VerticalAnchor::Centered,
                }
            }
            .resolve()
            .expect("resolves")
        };

        let below = resolve(DetailPlacement::BelowList);
        let detail = below.detail.expect("box");
        assert_eq!(detail.y, below.list.y + below.list.height);

        let above = resolve(DetailPlacement::AboveList);
        let detail = above.detail.expect("box");
        assert_eq!(detail.y, above.header.y + above.header.height);
        assert_eq!(detail.y + detail.height, above.list.y);
        assert_eq!(above.header_row(1).y, above.header.y + 1);

        let title = resolve(DetailPlacement::BelowTitle);
        let detail = title.detail.expect("box");
        assert_eq!(detail.y, title.header_row(0).y + 1, "right under the title");
        assert_eq!(
            title.header_row(1).y,
            detail.y + detail.height,
            "search under the box"
        );
        assert_eq!(title.header.y + title.header.height, title.list.y);
        assert_eq!(
            above.list.height, title.list.height,
            "same room for the list"
        );
    }

    #[test]
    fn detail_is_omitted_under_three_rows() {
        let with_detail = |detail_rows: u16, screen_height: u16| AnchoredPanelSpec {
            detail_rows,
            screen: Rect::new(0, 0, 80, screen_height),
            rows: 4,
            ..spec(VerticalAnchor::Below)
        };

        let roomy = with_detail(5, 40).resolve().expect("resolves");
        let detail = roomy.detail.expect("room for the box");
        assert_eq!(detail.height, 5);
        assert_eq!(detail.y, roomy.list.y + roomy.list.height);

        // Squeezed until only two rows could go to the detail: no lone rule.
        let squeezed = with_detail(5, 2 + 3 + crate::ui::FOOTER_ROWS)
            .resolve()
            .expect("resolves");
        assert!(squeezed.detail.is_none(), "{squeezed:?}");
        assert_eq!(squeezed.list.height, 3, "the list keeps the rows");

        // Exactly three rows is a box.
        let just = with_detail(5, 2 + 4 + crate::ui::FOOTER_ROWS)
            .resolve()
            .expect("resolves");
        assert_eq!(just.detail.map(|detail| detail.height), Some(3));

        // Asking for fewer than three is asking for none.
        assert!(with_detail(2, 40)
            .resolve()
            .expect("resolves")
            .detail
            .is_none());
    }

    #[test]
    fn width_clamps_between_its_bounds_then_to_the_screen() {
        let narrow = AnchoredPanelSpec {
            content_width: 4,
            ..spec(VerticalAnchor::Below)
        };
        assert_eq!(narrow.resolve().expect("resolves").outer.width, 30);

        let wide = AnchoredPanelSpec {
            content_width: 500,
            ..spec(VerticalAnchor::Below)
        };
        assert_eq!(wide.resolve().expect("resolves").outer.width, 60);

        let cramped = AnchoredPanelSpec {
            screen: Rect::new(0, 0, 20, 25),
            anchor: Rect::new(0, 0, 20, 1),
            ..wide
        };
        assert_eq!(cramped.resolve().expect("resolves").outer.width, 20);
    }

    #[test]
    fn rows_are_at_least_one_and_capped() {
        let empty = AnchoredPanelSpec {
            rows: 0,
            ..spec(VerticalAnchor::Below)
        };
        // one row, two borders, and the footer block
        assert_eq!(empty.resolve().expect("resolves").outer.height, 5);

        let many = AnchoredPanelSpec {
            rows: 40,
            ..spec(VerticalAnchor::Below)
        };
        assert_eq!(many.resolve().expect("resolves").outer.height, 16);
    }

    #[test]
    fn the_panel_right_aligns_to_its_anchor() {
        let geometry = AnchoredPanelSpec {
            anchor: Rect::new(0, 0, 50, 1),
            ..spec(VerticalAnchor::Below)
        }
        .resolve()
        .expect("resolves");
        assert_eq!(geometry.outer.x + geometry.outer.width, 50);

        // An anchor narrower than the panel would push it off the left edge.
        let clamped = AnchoredPanelSpec {
            anchor: Rect::new(0, 0, 10, 1),
            ..spec(VerticalAnchor::Below)
        }
        .resolve()
        .expect("resolves");
        assert_eq!(clamped.outer.x, SCREEN.x);
    }

    #[test]
    fn below_hangs_under_the_anchor_and_inside_top_drops_one_row_in() {
        let anchor = Rect::new(0, 4, 60, 10);
        let below = AnchoredPanelSpec {
            anchor,
            ..spec(VerticalAnchor::Below)
        }
        .resolve()
        .expect("resolves");
        assert_eq!(below.outer.y, 14);

        let inside = AnchoredPanelSpec {
            anchor,
            ..spec(VerticalAnchor::InsideTop)
        }
        .resolve()
        .expect("resolves");
        assert_eq!(inside.outer.y, 5);
    }

    #[test]
    fn above_opens_over_the_rect_that_toggles_it() {
        let indicator = Rect::new(70, 24, 5, 1);
        let above = spec(VerticalAnchor::Above(indicator))
            .resolve()
            .expect("resolves");
        assert_eq!(above.outer.y + above.outer.height, indicator.y);

        // Nothing to open above: flush at the bottom instead.
        let flush = spec(VerticalAnchor::Above(Rect::default()))
            .resolve()
            .expect("resolves");
        assert_eq!(flush.outer.y + flush.outer.height, SCREEN.height);
    }

    #[test]
    fn a_panel_that_would_fall_off_the_bottom_is_pushed_up() {
        let geometry = AnchoredPanelSpec {
            anchor: Rect::new(0, 23, 60, 1),
            ..spec(VerticalAnchor::Below)
        }
        .resolve()
        .expect("resolves");
        assert_eq!(geometry.outer.y + geometry.outer.height, SCREEN.height);
    }

    #[test]
    fn inner_list_and_footer_partition_the_panel() {
        let geometry = spec(VerticalAnchor::Below).resolve().expect("resolves");
        let outer = geometry.outer;
        assert_eq!(
            geometry.inner,
            Rect::new(outer.x + 1, outer.y + 1, outer.width - 2, outer.height - 2)
        );
        let footer = geometry.footer_row.expect("footer row");
        assert_eq!(footer.y, geometry.inner.y + geometry.inner.height - 1);
        // The list stops a blank row short of the buttons.
        assert_eq!(
            geometry.list.y + geometry.list.height + crate::ui::FOOTER_ROWS,
            geometry.inner.y + geometry.inner.height
        );
        assert_eq!(geometry.list.y + geometry.list.height, footer.y - 1);
    }

    #[test]
    fn a_panel_with_no_footer_gives_the_whole_inner_area_to_its_list() {
        let geometry = AnchoredPanelSpec {
            footer_rows: 0,
            ..spec(VerticalAnchor::Below)
        }
        .resolve()
        .expect("resolves");
        assert_eq!(geometry.list, geometry.inner);
        assert!(geometry.footer_row.is_none());
    }

    /// Too short for the whole footer block: the buttons are dropped rather
    /// than drawn flush against the last row, and the list keeps the space.
    #[test]
    fn a_footer_that_cannot_fit_its_block_is_not_drawn() {
        let geometry = AnchoredPanelSpec {
            screen: Rect::new(0, 0, 80, 3),
            ..spec(VerticalAnchor::Below)
        }
        .resolve()
        .expect("resolves");
        assert_eq!(geometry.inner.height, 1);
        assert!(geometry.footer_row.is_none());
        assert_eq!(geometry.list, geometry.inner);
    }

    #[test]
    fn a_panel_with_no_screen_resolves_to_nothing() {
        assert!(AnchoredPanelSpec {
            screen: Rect::new(0, 0, 0, 25),
            ..spec(VerticalAnchor::Below)
        }
        .resolve()
        .is_none());
        assert!(AnchoredPanelSpec {
            screen: Rect::new(0, 0, 80, 0),
            ..spec(VerticalAnchor::Below)
        }
        .resolve()
        .is_none());
    }
}
