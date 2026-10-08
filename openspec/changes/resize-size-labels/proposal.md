## Why

https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/122, asked by ac
2026-09-29 and approved as designed ("B ok"). Resizing a pane shows no sizes;
to read them you press `prefix+i` (display panes) after the fact.

## What Changes

- While a pane is being resized, every pane of the visible tab shows the same
  label as display panes, without the digit (digits do nothing here):
  `w5:p16 · claude  138x27`. The numbers follow every resize step.
- **Mouse:** while a split border is dragged, and for 1 s after the release.
  Any key or click hides them at once.
- **Keyboard resize mode** (`Mode::Resize`): for the whole mode, gone on esc.
  The `RESIZE` mode bar stays as it is.
- **A single resize key** (a direct `resize_pane_*` binding outside resize
  mode): for 1 s.
- The labels are a passive layer drawn over whatever mode is on, not
  `Mode::DisplayPanes`, so they never take input from the drag or the resize
  mode.
- Not included: labels for the sidebar divider drag.

## Capabilities

### Modified Capabilities

- `display-panes`: the same labels also show during a resize.

## Impact

- `src/app/display_panes.rs`: a `resize_labels_until` presentation deadline on
  `AppState` (TUI/client state, no API or protocol change) and the rule that
  decides when labels show.
- `src/app/input/mouse.rs`, `navigate.rs`, `modal.rs`: arm the deadline on a
  split drag step and release, on each resize key; clear it on any other key or
  click.
- `src/ui/display_panes.rs` / `src/ui.rs`: draw the labels (label part only, no
  summary bar) when the rule says so; the loop wakes at the deadline.
- Performance: labels come from `view.pane_infos` of the visible tab, already
  computed. When no resize is happening the cost is one check per frame; during
  a resize, one small string per visible pane per frame.
