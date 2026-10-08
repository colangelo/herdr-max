//! The overlay kit: the geometry, rows, and cursors every panel, modal, and
//! picker is built from.
//!
//! Ratatui deliberately ships no overlay or component model, so a ratatui app
//! grows its own layer there. Herdr's grew by copy-adapt until two panels were
//! running byte-for-byte parallel arithmetic. This module is that arithmetic,
//! written once.
//!
//! Nothing here holds mutable widget state: `compute_view` mutates, `render`
//! does not, and the kit is data and geometry rather than a widget tree.

mod button_row;
mod detail;
mod geometry;
mod list_cursor;
mod search;

pub(crate) use button_row::{ButtonRow, ButtonRowHit, ButtonSpec};
pub(crate) use detail::{
    detail_box_rows, render_detail_box, DETAIL_MAX_TEXT_ROWS, TODO_DETAIL_PLACEMENT,
};
pub(crate) use geometry::{
    AnchoredPanelSpec, DetailPlacement, PanelGeometry, VerticalAnchor, LIST_DIALOG_MAX_WIDTH,
};
pub(crate) use list_cursor::{reveal_scroll, ListCursor};
pub(crate) use search::{
    render_search_row, set_search_caret, text_matches_query, ListSearch, SearchKey, SearchRow,
};
