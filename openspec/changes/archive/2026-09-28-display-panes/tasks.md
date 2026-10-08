## 1. Overlay

- [x] 1.1 Tests first: rendered-layout snapshot, key handling (open, dismiss, focus by number, timeout)
- [x] 1.2 `DisplayPanes` overlay, state, deadline and expiry in both loops
- [x] 1.3 Label and mode-bar rendering

## 2. Binding

- [x] 2.1 `keys.display_panes` (default `prefix+i`), help entry, config template
- [x] 2.2 Docs: `keyboard.mdx`, config reference data

## 3. Ship

- [x] 3.1 `just check` green
- [x] 3.2 Beta; proved on m4m

Verified 2026-09-28 on m4m with the `0.8.2-ac-beta.103-morata` release binary in an isolated `--session` inside a private tmux, with ac's config (prefix `ctrl+s`): `prefix+i` labelled three panes (`1  w1:p1 · pane 1  64x37` and so on) and the bar read `window 160x40 · panes 134x39`; `2` focused pane 2, whose shell reported `cols=64 lines=18`, the size on its label; the labels closed on their own after 3 s.
