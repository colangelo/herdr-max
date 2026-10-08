Planned 2026-09-29 (JOB 14 from herdr-helper; ac approved the design step for
https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/126). Code waits for
approval of this change (approved: whole-screen hold).

## 1. Tests first (red)

- [x] 1.1 terminal: epoch changes on begin and end only; begin returns a render delay of `SYNC_HOLD_MAX` (`src/pane/terminal.rs`)
- [x] 1.2 helper: mid-block pane in the target holds; past the deadline draws; hidden tab does not hold; popup counts
- [x] 1.3 server: full render holds and owes a frame; the end sends it; epoch change during the build drops the frame; retained patch falls back (`src/server/headless.rs` tests)

## 2. Build (green)

- [x] 2.1 epoch + begin instant in `GhosttyPaneCore`, accessor through `PaneRuntime` / `TerminalRuntime`
- [x] 2.2 the helper, used by full render, retained path and local loop
- [x] 2.3 hold + owed render in the headless per-client loop

## 3. Verify

- [x] 3.1 `just bench-render-scale` 1 vs 15 panes, before/after, numbers on the issue
- [x] 3.2 #126 reproducer against the beta: torn-at-middle with keys goes to 0
- [x] 3.3 `just check`, push origin and internal
