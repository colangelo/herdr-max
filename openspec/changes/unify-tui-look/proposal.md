## Why

https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/174, asked by ac on 2026-10-06: "we
need to unify the user interface, and make it consistent and beautiful! the asks pane seems the
one that needs more work. check the upstream ui design docs or issues if there are". On
2026-10-08 he added: "there's worker waiting, shouldnt they try to fix the asks pane as per ui
rules, given it's a separate program".

Herdr's overlays share one panel: a blue frame, a bold title, a dim subtitle and a blue selection
bar. The asks pane is a separate Python curses program (`CONTEXT/SKILLS/gestore/asks_tui.py`) that
runs in a herdr popup, and it looks unrelated. It uses a reverse-video selection with `❯`, a
`filter:` string on the title row, and white key labels. Upstream has no UI design document, and
its issues set no direction. Even its 0.9.3 footers come in three styles. So the rules have to be
written down before anyone restyles anything.

## What Changes

- Write herdr's UI language down as a short style spec in `design.md`. It covers the palette (with
  256-colour numbers for curses), frame, title, rules, selection, state colours, footer, spacing,
  truncation, and empty or error states. It is taken from nine screenshots, the fork's drawing
  code, and the upstream 0.9.3 client shell.
- Mock the asks pane BEFORE and AFTER (list view, answering view, typing, bus down), plus two
  herdr screens that clash: the PANES bar and the todos/notes panel.
- Phase 1 (this change): rules and mockups only. No code.
- Phase 2, after ac's OK: restyle `asks_tui.py` to the rules on the CONTEXT branch
  `feat/asks-tui-herdr-style`. Its owner, `context`, reviews and merges it.
- Phase 3, after the 0.9.3 cutover and ac's OK: fix the two herdr clashes.
- Kept as ac chose: light blue (256-colour 147) for option labels and descriptions, and herdr-style
  key bars (key then label). The mockups show the two places where these get restyled. The key
  colour becomes blue and labels become grey. The option under the cursor becomes the blue bar.

## Capabilities

### New Capabilities

- `tui-style`: the shared look of herdr overlays and of programs that run in a herdr popup.

### Modified Capabilities

None.

## Impact

- Phase 1: only `openspec/changes/unify-tui-look/` in this repo.
- Phase 2: `SKILLS/gestore/asks_tui.py` and `SKILLS/gestore/tests/test_asks_tui.py` in the CONTEXT
  repo. `asks_page/ANSWER-UI.md` §1/§3 change only if a key or a footer label changes, and none
  does in this design.
- Phase 3: `src/ui/display_panes.rs` and `src/ui/todo_board.rs`, or wherever the #171 port puts
  them under `src/client/shell`.
