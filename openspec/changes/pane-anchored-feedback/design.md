## Context

Copy feedback is `AppState.copy_feedback` (message only), placed by
`copy_feedback_rect` in `src/ui/status.rs`. herdr toasts are
`AppState.toast` (`ToastNotification`), placed by `toast_notification_rect`.
Both are drawn by `render_notifications` in `src/ui.rs`.

## Decisions

### Carry the pane with the feedback, resolve the rect at render time

The feedback stores the pane id only. Render looks the pane up in
`view.pane_infos` (the visible tab's panes, already computed for the frame)
and uses its inner rect. So a pane that moved, resized, left the tab or closed
while the box is up falls back on the next frame with no extra bookkeeping.

### Config shape

- Clipboard: a new `position` value, `"pane"`, next to the six screen spots:
  the box has one placement setting and this is one more place.
- Pane notes: a separate `pane_feedback = "corner" | "pane"` under
  `[ui.toast.herdr]`, because `position` there also places agent
  notifications, which must stay in the corner.

### Fallback

Pane placement is used only when the pane is shown and its inner rect holds
the whole box (clipboard: message + 4 columns by 3 rows; notes: the toast's
own size). Otherwise the box goes where it goes today: the clipboard box at
`bottom-center` with the existing offset against a herdr toast, the note in
the `[ui.toast.herdr]` position.

### Which pane a note is about

`set_pane_move_feedback` runs right after an action on the focused pane, so it
records the focused pane of the active workspace. A note with no focused pane
has no anchor and uses the corner.

### Overlap

A pane-placed clipboard box is not moved for a corner toast (they are in
different places); if both are pane-placed in the same pane, the clipboard box
is drawn one box height under the note.

## Testing

- `status.rs`: pane placement centers in the pane; too small, missing or
  hidden pane falls back.
- Producers: OSC 52 and herdr copies carry the pane; the note records the
  focused pane.
- Rendered-layout test: a copy with `position = "pane"` draws the box inside
  the source pane; with the default it stays at the bottom center.
- Config: both keys parse, defaults unchanged, reference check passes.
