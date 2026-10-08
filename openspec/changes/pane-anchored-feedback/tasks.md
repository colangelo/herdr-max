Planned 2026-09-29 (JOB 14; ac: "your pick, included", comment 17422 on
https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/129).

## 1. Tests first (red)

- [x] 1.1 placement: centered in the pane; fallback when too small, missing, hidden
- [x] 1.2 producers: OSC 52 and herdr copies carry the pane; a note records the focused pane
- [x] 1.3 rendered layout: pane placement vs default; config parse and defaults

## 2. Build (green)

- [x] 2.1 `ClipboardWrite.source_pane`, copy request pane, `CopyFeedback.source_pane`
- [x] 2.2 `ToastNotification.anchor_pane` set by `set_pane_move_feedback`
- [x] 2.3 config keys, template, reference, docs; placement in `render_notifications`

## 3. Verify

- [x] 3.1 throwaway: copy in a pane with `position = "pane"`, capture the screen
- [x] 3.2 `just check`, push, notes on #129
