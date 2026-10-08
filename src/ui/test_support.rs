//! Shared helpers for rendered-layout ("snapshot") tests of overlays.
//!
//! The kit's promise is that no overlay moves, which is what a rendered buffer
//! proves and what a unit test on the arithmetic does not. Every overlay gets a
//! test here-shaped: render the app twice, once with the overlay open and once
//! without, take the bounding box of the cells that changed, and assert both
//! that box and the text inside it.
//!
//! Diffing against a closed render rather than snapshotting the whole frame is
//! deliberate. It pins the overlay's *placement* — the box moving is a failure —
//! without making every overlay test a hostage to an unrelated sidebar or tab
//! bar tweak.

use ratatui::{backend::TestBackend, buffer::Buffer, layout::Rect, Frame, Terminal};

use crate::app::state::AppState;

/// The frame every overlay snapshot is taken at. Fixed on purpose: a snapshot
/// at a size the test picked per case proves nothing about the next case.
pub(crate) const SNAPSHOT_WIDTH: u16 = 80;
pub(crate) const SNAPSHOT_HEIGHT: u16 = 25;

/// Lay out `app` for the snapshot frame, so `view` geometry is the geometry the
/// renderer and the mouse hit-test would both see.
pub(crate) fn layout(app: &mut AppState) {
    layout_sized(app, SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT);
}

pub(crate) fn layout_sized(app: &mut AppState, width: u16, height: u16) {
    super::compute_view(app, Rect::new(0, 0, width, height));
}

/// Render the whole UI at a fixed size.
pub(crate) fn draw_sized(app: &AppState, width: u16, height: u16) -> Buffer {
    draw_with(width, height, |frame| super::render(app, frame))
}

/// Render an arbitrary draw closure, for widgets that are not reached through
/// [`super::render`].
pub(crate) fn draw_with(width: u16, height: u16, draw: impl FnOnce(&mut Frame)) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("test backend should build");
    terminal.draw(draw).expect("draw should succeed");
    terminal.backend().buffer().clone()
}

/// The text of `rect`'s first row, exactly as drawn.
pub(crate) fn row_text(buffer: &Buffer, rect: Rect) -> String {
    (rect.x..rect.x.saturating_add(rect.width))
        .map(|x| buffer[(x, rect.y)].symbol())
        .collect()
}

/// [`row_text`] with trailing blanks dropped, for rows whose padding is not the
/// point of the assertion.
pub(crate) fn row_text_trimmed(buffer: &Buffer, rect: Rect) -> String {
    row_text(buffer, rect).trim_end().to_string()
}

/// Every row of `rect`, trailing blanks dropped.
pub(crate) fn rect_rows(buffer: &Buffer, rect: Rect) -> Vec<String> {
    (rect.y..rect.y.saturating_add(rect.height))
        .map(|y| row_text_trimmed(buffer, Rect::new(rect.x, y, rect.width, 1)))
        .collect()
}

/// The bounding box of the cells whose text differs between two renders of the
/// same frame — the overlay's footprint. `None` when nothing moved.
pub(crate) fn changed_rect(base: &Buffer, open: &Buffer) -> Option<Rect> {
    let area = open.area;
    assert_eq!(base.area, area, "snapshots must be taken at the same size");
    let mut left = u16::MAX;
    let mut top = u16::MAX;
    let mut right = 0u16;
    let mut bottom = 0u16;
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if base[(x, y)].symbol() != open[(x, y)].symbol() {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    (left != u16::MAX).then(|| Rect::new(left, top, right - left, bottom - top))
}

/// An overlay's footprint and the text inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OverlaySnapshot {
    pub rect: Rect,
    pub rows: Vec<String>,
}

/// Render `base` and `open` and describe what the overlay put on screen.
pub(crate) fn overlay_snapshot(base: &AppState, open: &AppState) -> OverlaySnapshot {
    overlay_snapshot_sized(base, open, SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT)
}

pub(crate) fn overlay_snapshot_sized(
    base: &AppState,
    open: &AppState,
    width: u16,
    height: u16,
) -> OverlaySnapshot {
    let base = draw_sized(base, width, height);
    let open = draw_sized(open, width, height);
    let rect = changed_rect(&base, &open).expect("an open overlay should change the frame");
    OverlaySnapshot {
        rows: rect_rows(&open, rect),
        rect,
    }
}

impl OverlaySnapshot {
    /// Assert the footprint and its text. The failure prints the actual
    /// snapshot in the literal form the assertion takes, so an intended move is
    /// re-recorded by pasting rather than by hand-editing rows.
    #[track_caller]
    pub(crate) fn assert(&self, rect: Rect, rows: &[&str]) {
        let expected: Vec<String> = rows.iter().map(|row| (*row).to_string()).collect();
        if self.rect != rect || self.rows != expected {
            panic!("overlay snapshot changed\n{}", self.as_literal());
        }
    }

    pub(crate) fn as_literal(&self) -> String {
        let mut out = format!(
            "Rect::new({}, {}, {}, {}),\n&[\n",
            self.rect.x, self.rect.y, self.rect.width, self.rect.height
        );
        for row in &self.rows {
            out.push_str(&format!("    {row:?},\n"));
        }
        out.push_str("],\n");
        out
    }
}

/// A laid-out app with one workspace and one pane, in `Mode::Terminal` — the
/// quiet background every overlay snapshot is diffed against.
pub(crate) fn app_with_one_pane(name: &str) -> AppState {
    let mut app = AppState::test_new();
    app.workspaces = vec![crate::workspace::Workspace::test_new(name)];
    app.active = Some(0);
    app.selected = 0;
    app.mode = crate::app::state::Mode::Terminal;
    app.ensure_test_terminals();
    layout(&mut app);
    app
}

/// Snapshot an overlay opened by `open` against the quiet one-pane background.
pub(crate) fn overlay_snapshot_of(open: impl FnOnce(&mut AppState)) -> OverlaySnapshot {
    let base = app_with_one_pane("overlay");
    let mut opened = app_with_one_pane("overlay");
    open(&mut opened);
    layout(&mut opened);
    overlay_snapshot(&base, &opened)
}
