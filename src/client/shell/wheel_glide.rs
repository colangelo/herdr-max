//! Eased wheel scrolling (`ui.smooth_scroll`).
//!
//! A host terminal only shows a cell grid, so pane scrollback cannot move by
//! pixels. Instead of sending a notch's `mouse_scroll_lines` rows in one wheel
//! event, the client keeps them pending and sends them as a few smaller wheel
//! events a frame apart: a quarter of what is left each step, at least one
//! row. A burst catches up fast and every glide ends one row at a time.
//!
//! Only wheel events that scroll host scrollback are paced. A pane whose app
//! takes the wheel (mouse reporting, or the alternate screen) turns every
//! wheel event into one report or arrow key whatever `lines` says, so pacing
//! it would multiply the scroll instead of spreading it.

use super::*;
use crate::protocol::{ClientMouseGeometry, ClientMouseKind};
use crossterm::event::{MouseEvent, MouseEventKind};

/// Time between two glide steps: about one frame at 120 Hz.
pub(super) const WHEEL_GLIDE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(8);

/// Each step sends `1 / WHEEL_GLIDE_EASE_DIVISOR` of the pending rows.
const WHEEL_GLIDE_EASE_DIVISOR: usize = 4;

/// Rows the next glide step sends when `pending` rows are left: a quarter,
/// rounded up, so the step never stalls and the last steps are single rows.
pub(super) fn wheel_glide_step(pending: usize) -> usize {
    pending.div_ceil(WHEEL_GLIDE_EASE_DIVISOR)
}

/// The rows still to send for the pane being scrolled, and the wheel event
/// they are sent as.
pub(super) struct ClientWheelGlide {
    pane_id: String,
    inner_rect: Rect,
    kind: ClientMouseKind,
    position: ClientMousePosition,
    geometry: Option<ClientMouseGeometry>,
    modifiers: u8,
    pending: usize,
}

impl ClientShellState {
    /// Takes a wheel notch over `hit` into the glide when `ui.smooth_scroll`
    /// applies to it. Returns false when the caller must send the wheel event
    /// unchanged.
    pub(super) fn glide_pane_wheel(
        &mut self,
        hit: &PaneHit,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) -> bool {
        if !self.config.smooth_scroll || !self.wheel_glide_applies(hit) {
            return false;
        }
        let kind = match mouse.kind {
            MouseEventKind::ScrollUp => ClientMouseKind::ScrollUp,
            MouseEventKind::ScrollDown => ClientMouseKind::ScrollDown,
            _ => return false,
        };
        let position = self.pane_mouse_position(hit, mouse);
        let carried = match self.wheel_glide.take() {
            Some(glide) if glide.pane_id == hit.pane_id => {
                // A direction change drops the rows still pending the other way.
                if glide.kind == kind {
                    glide.pending
                } else {
                    0
                }
            }
            Some(glide) => {
                // Another pane: finish the old glide at once so no rows are lost.
                send_wheel_rows(&glide, glide.pending, outcome);
                0
            }
            None => 0,
        };
        let gliding = carried > 0 && self.wheel_glide_deadline.is_some();
        self.wheel_glide = Some(ClientWheelGlide {
            pane_id: hit.pane_id.clone(),
            inner_rect: hit.inner_rect,
            kind,
            position,
            geometry: pane_mouse_geometry(hit, position),
            modifiers: mouse.modifiers.bits(),
            pending: carried.saturating_add(self.config.mouse_scroll_lines),
        });
        if !gliding {
            // A new glide moves at once; a running one keeps its own pace.
            self.step_wheel_glide(std::time::Instant::now(), outcome);
        }
        true
    }

    pub(crate) fn tick_wheel_glide(
        &mut self,
        now: std::time::Instant,
        outcome: &mut ClientShellInput,
    ) {
        if self
            .wheel_glide_deadline
            .is_none_or(|deadline| now < deadline)
        {
            return;
        }
        let still_applies = self.config.smooth_scroll
            && self.wheel_glide.as_ref().is_some_and(|glide| {
                self.hits.panes.iter().any(|hit| {
                    hit.pane_id == glide.pane_id
                        && hit.inner_rect == glide.inner_rect
                        && self.wheel_glide_applies(hit)
                })
            });
        if still_applies {
            self.step_wheel_glide(now, outcome);
        } else {
            self.stop_wheel_glide();
        }
    }

    pub(super) fn stop_wheel_glide(&mut self) {
        self.wheel_glide = None;
        self.wheel_glide_deadline = None;
    }

    fn step_wheel_glide(&mut self, now: std::time::Instant, outcome: &mut ClientShellInput) {
        let Some(glide) = self.wheel_glide.as_mut() else {
            self.wheel_glide_deadline = None;
            return;
        };
        let rows = wheel_glide_step(glide.pending).min(usize::from(u16::MAX));
        glide.pending -= rows;
        send_wheel_rows(glide, rows, outcome);
        if glide.pending == 0 {
            self.stop_wheel_glide();
        } else {
            self.wheel_glide_deadline = Some(now + WHEEL_GLIDE_INTERVAL);
        }
    }

    /// The wheel over `hit` scrolls host scrollback, so `lines` is honoured.
    fn wheel_glide_applies(&self, hit: &PaneHit) -> bool {
        !hit.popup
            && !hit.mouse_reporting
            && !self.pane_surface.as_ref().is_some_and(|surface| {
                surface
                    .panes
                    .iter()
                    .any(|pane| pane.pane_id == hit.pane_id && pane.alternate_screen_active)
            })
    }
}

fn send_wheel_rows(glide: &ClientWheelGlide, rows: usize, outcome: &mut ClientShellInput) {
    if rows == 0 {
        return;
    }
    push_target_event(
        ClientInputTarget::Pane(glide.pane_id.clone()),
        ClientPaneInputEvent::Mouse {
            kind: glide.kind,
            position: glide.position,
            geometry: glide.geometry,
            modifiers: glide.modifiers,
            lines: rows.min(usize::from(u16::MAX)) as u16,
        },
        outcome,
    );
}

/// The pixel geometry a pixel-position mouse event carries, or none for a
/// cell position.
pub(super) fn pane_mouse_geometry(
    hit: &PaneHit,
    position: ClientMousePosition,
) -> Option<ClientMouseGeometry> {
    matches!(position, ClientMousePosition::Pixels { .. }).then_some(ClientMouseGeometry {
        cols: hit.inner_rect.width,
        rows: hit.inner_rect.height,
        width_px: hit.pixel_width,
        height_px: hit.pixel_height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(mut pending: usize) -> Vec<usize> {
        let mut steps = Vec::new();
        while pending > 0 {
            let rows = wheel_glide_step(pending);
            assert!(rows >= 1 && rows <= pending);
            pending -= rows;
            steps.push(rows);
        }
        steps
    }

    #[test]
    fn wheel_glide_steps_sum_to_pending() {
        for pending in 0..=200 {
            assert_eq!(drain(pending).iter().sum::<usize>(), pending);
        }
    }

    #[test]
    fn wheel_glide_eases_out() {
        assert_eq!(drain(3), [1, 1, 1]);
        assert_eq!(drain(12), [3, 3, 2, 1, 1, 1, 1]);
        for pending in 1..=200 {
            let steps = drain(pending);
            assert!(
                steps.windows(2).all(|pair| pair[0] >= pair[1]),
                "steps never grow: {steps:?}"
            );
            assert_eq!(steps.last(), Some(&1), "a glide ends on one row");
        }
        // A big burst catches up in a handful of frames, not one per row.
        assert!(drain(60).len() <= 15);
    }

    #[test]
    fn wheel_glide_step_of_nothing_is_nothing() {
        assert_eq!(wheel_glide_step(0), 0);
    }
}
