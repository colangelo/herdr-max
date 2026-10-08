Planned 2026-10-08. Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/174

## 1. Phase 1: rules and mockups (this change, herdr repo, no code)

- [x] 1.1 read the 9 screenshots in `~/Pictures/Screenshots/herdr-tui - 2026-10-06`, the fork's `src/ui`, upstream 0.9.3 `src/client/shell/{overlays,render}.rs`, `asks_tui.py`, and upstream issues
- [x] 1.2 write the style spec and the BEFORE/AFTER mockups in `design.md` (asks list, answering, typing, bus down, PANES bar, todos/notes)
- [x] 1.3 `openspec validate unify-tui-look --strict`
- [ ] 1.4 ac approves the mockups, through herdr-helper and the coordinator `herdr`; record his answer on #174

## 2. Phase 2: asks pane (CONTEXT repo, branch `feat/asks-tui-herdr-style`, `context` reviews and merges)

- [ ] 2.1 make the branch: `wt switch --create feat/asks-tui-herdr-style --base main --no-cd` in `~/_sync/dev/CONTEXT`; never push or merge `main`
- [ ] 2.2 `build_attrs`: add `accent` (111), `bar` (bg 111, fg 234, bold), `dimc` (243), `rule` (239), `text` (189), `field` (bg 237); remap `ok`/`warn`/`bad` to 151/223/211; keep `choice` = 147; keep the no-colour fallbacks (bar → reverse, dim → `A_DIM`)
- [ ] 2.3 `footer`: keys in bold `accent`, labels dim, two spaces; a status message uses the blank row above the bar; `enter` dims in read-only mode
- [ ] 2.4 `render_list`: `asks` title with dim count on the right; dim summary row (active filter parts in accent); `surface1` rule inset one column; no column-header row; rows = mark · header · title on the left, own · key · age · turn dim on the right; accent bar selection, no `❯`; marks `☐` text, `✔` green, `!` yellow, `·` dim; narrow drop order turn → owner → key → cut the title
- [ ] 2.5 bus banner: red label at the start of the rule row; yellow read-only status row
- [ ] 2.6 `render_answer`: same title row with the position on the right; chips row with the active chip in accent; rules in `surface1`; option under the cursor = accent bar, others 147; `more lines` hint dim; typing field on a `surface0` strip
- [ ] 2.7 tests in `tests/test_asks_tui.py`: update the text expectations; add a style test for the bar, the key bar and the banner; keep the full-width repaint of every row (829ec84), cell widths through `cells()`/`clip`, typed answers (`Type something.`, Alt+Enter, Esc keeps the draft), the bus-down read-only path, and the deleted-while-shown path
- [ ] 2.8 oracle in CONTEXT: `just check-asks` and `just check-asks-page` (the counts `context` gave: 341 + 1 skipped, and 57), `just check` with 0 errors; no change to `asks_page/ANSWER-UI.md` unless a key or footer label changed
- [ ] 2.9 ac looks at the live popup (`prefix+ctrl+a`) on m4m; `context` merges; note the CONTEXT SHA on #174

## 3. Phase 3: herdr clashes (after the 0.9.3 cutover, and only with ac's OK)

- [ ] 3.1 open a `[fork]` issue for each item below before starting it
- [ ] 3.2 PANES bar (`display_panes.rs`, or its `src/client/shell` successor after #171): `PANES` chip in `accent`, keys in `accent`, `version` as a dim label (no second chip); update its tests, including the one that says the `VERSION` chip is styled like `PANES`
- [ ] 3.3 todos/notes panel (`todo_board.rs` or its successor): count on the title row, search as the subtitle, no inner frame, preview under the list after a rule, `↵ open pane` as the accent primary chip
- [ ] 3.4 optional: bring the 0.9.3 help and navigator footers to the key bar rule (keys first, bold accent) and drop hints that do nothing (upstream #2880); shape it so it can go upstream
- [ ] 3.5 dogfood each fix with `herdr-dogfood`, then close its issue with how it was checked
