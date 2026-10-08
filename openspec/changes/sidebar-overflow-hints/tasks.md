https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/159

## 1. Setting

- [x] 1.1 `SidebarOverflowConfig`, `ui.sidebar_overflow`, startup and reload wiring, `--default-config` entry, config reference

## 2. Layout and plan

- [x] 2.1 tests first: window and last-window arithmetic, summaries, fog colours, plan
- [x] 2.2 `ui/sidebar_overflow.rs`; both lists lay out inside the window

## 3. Draw and click

- [x] 3.1 rendered-layout tests: each mode, both panels, top and bottom, tint, exemption
- [x] 3.2 `render_overflow`; click on an edge row scrolls a page

## 4. Docs and proof

- [x] 4.1 `configuration.mdx`, README lists
- [ ] 4.2 live proof on a throwaway: each mode captured with `capture-pane -e`, fog values
