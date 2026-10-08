## Why

https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/126 (ac approved
2026-09-29, JOB 14). An app that draws with synchronized output (DEC mode
2026: begin, draw, end) can be shown half old frame and half new frame.

Measured on the fork (beta 0.8.2-ac-beta.110, throwaway session, 160x45): a
program that fills its pane with one letter per frame, writing the top half,
waiting 30 ms and writing the bottom half inside each 2026 block, is shown torn
in **38% of screen samples (708 of 1849) while keys are sent**, 702 of them
split exactly at the middle. With no input it is 0.33%, at random rows (outer
tmux noise). Without 2026 at all it is 58%.

Cause: the fork already drops the render that the pane's own output asks for
while its block is open (`src/pane/terminal.rs`, `request_render =
!synchronized_output`). But any other render (a key, another pane's output, a
timer, a resize) draws every visible pane from its current grid, and a pane in
the middle of its block has a half-updated grid.

Upstream fixed the same class in 440a8bdd ("keep synchronized pane frames
atomic", herdrdev/herdr#4508) inside `src/server/client_shell.rs`, a renderer
the fork does not have, so it does not cherry-pick.

## What Changes

- **A frame is never presented while a visible pane is mid-block.** Before a
  client frame is built, herdr checks the panes that frame shows (the visible
  tab's panes and the popup pane). If one is inside an open synchronized
  update, the frame for that client is held: nothing is sent, the client keeps
  showing its last complete frame, and a render is owed.
- **The block's end draws right away.** The end of a block already requests a
  render (unchanged), so the held frame goes out as soon as the app finishes.
- **A stuck app cannot freeze the screen.** A block older than
  `SYNC_HOLD_MAX` (200 ms) no longer holds frames; herdr draws the pane as it
  is, as today. A block's begin schedules a render at that deadline, so the
  timeout needs no input to fire.
- **A block that starts during the build is caught.** Each pane carries a
  synchronized-output epoch that changes on every begin and end. If an epoch
  shown in the frame changed between the check and the end of the build, the
  frame is dropped and redone (upstream's technique).
- **The retained dirty-patch path falls back** to the full render (which then
  holds) when a pane it would patch is mid-block, so a patch never carries half
  a frame.
- The local (non-server) TUI loop uses the same check before `terminal.draw`.

Not changed: a pane's own renders during its block stay suppressed; hidden
panes (other tabs, other workspaces) are never checked and never hold anything.

## Capabilities

### New Capabilities

- `synchronized-pane-frames`: when herdr may present a pane that uses
  synchronized output.

## Impact

- `src/pane/terminal.rs`: epoch + begin instant in `GhosttyPaneCore`, updated
  in `process_pty_bytes` and resize; `synchronized_output_state()` accessor;
  render delay of `SYNC_HOLD_MAX` on a begin.
- `src/pane.rs`, `src/terminal/runtime.rs`: pass-through accessors.
- `src/server/render_stream.rs` / `src/ui`: one pure helper,
  "does this frame show a pane that holds it", used by the server's full render,
  the retained patch path and the local loop.
- `src/server/headless.rs`: hold + owed render per client.
- Performance: one extra `(bool, epoch, instant)` read per visible pane per
  frame (same lock the cursor code already takes for
  `synchronized_output_active`), nothing for hidden panes. Profiled with
  `just bench-render-scale` at 1 and 15 panes.
