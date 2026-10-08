Planned 2026-09-29 (JOB 14; ac: "B ok"). Issue:
https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/122

## 1. Tests first (red)

- [x] 1.1 state rule: drag, release + 1 s, key clears, resize mode, single resize key
- [x] 1.2 render: labels over resize mode, no summary bar, visible tab only

## 2. Build (green)

- [x] 2.1 `resize_labels_until` + `resize_labels_visible` in `src/app/display_panes.rs`, wake-up deadline
- [x] 2.2 arm/clear from mouse, navigate and modal input
- [x] 2.3 draw the label layer in `src/ui.rs`

## 3. Verify

- [x] 3.1 `just bench-render-scale` 1 vs 15 panes
- [x] 3.2 throwaway: drag a border and use resize mode, capture the screen
- [x] 3.3 `just check`, push, notes on #122
