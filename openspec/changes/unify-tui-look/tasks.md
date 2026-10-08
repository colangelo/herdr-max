Planned 2026-10-08. Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/174

## 1. Phase 1: rules and mockups (this change, herdr repo, no code)

- [x] 1.1 read the 9 screenshots in `~/Pictures/Screenshots/herdr-tui - 2026-10-06`, the fork's `src/ui`, upstream 0.9.3 `src/client/shell/{overlays,render}.rs`, `asks_tui.py`, and upstream issues
- [x] 1.2 write the style spec and the BEFORE/AFTER mockups in `design.md` (asks list, answering, typing, bus down, PANES bar, todos/notes)
- [x] 1.3 `openspec validate unify-tui-look --strict`
- [x] 1.4 ac approves the mockups, through the coordinator `herdr`, on 2026-10-08: *"i like the new look, but for an alt-modality like the one in D i prefer it in red."* and *"keep your design but in that modality use the current red (that's not intese) where it's blue"*
- [x] 1.5 apply ac's change: mode bars take their mode's colour (PANES = soft red `#f38ba8`/211); decision 9, the Footer rule, mockup D and the spec updated
- [x] 1.6 apply ac's add-on, 2026-10-08: *"there is also the sync mode that is yellow, right? there could be a green one maybe, an orange, it depends, but in that case the color should be kept"*. Added the general Mode colour rule and the mode → colour table (SYNC keeps `#ffd60a`), and noted SYNC under mockup D

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
- [ ] 3.2 PANES bar (`display_panes.rs`, or its `src/client/shell` successor after #171): one `PANES` chip in the current soft red (`palette.red`) with dark bold text, keys in bold red, `version` as a dim label (no second chip); update its tests, including the one that says the `VERSION` chip is styled like `PANES`
- [ ] 3.3 todos/notes panel (`todo_board.rs` or its successor): count on the title row, search as the subtitle, no inner frame, preview under the list after a rule, `↵ open pane` as the accent primary chip
- [ ] 3.4 optional: bring the 0.9.3 help and navigator footers to the key bar rule (keys first, bold accent) and drop hints that do nothing (upstream #2880); shape it so it can go upstream
- [ ] 3.5 dogfood each fix with `herdr-dogfood`, then close its issue with how it was checked

### Phase 3 file map (checked 2026-10-08 against `sync/merge` @ 7670867d, v0.9.3 + port, and `port/chrome`/`port/overlays`)

Paths are `src/client/shell/` unless noted. **Clash:** commit 8a5d2b91 ("strip client-only fork code") removed `src/ui/display_panes.rs`, `menus.rs`, `navigator.rs`, `todo_panel.rs` from the port trees, so the PANES / RESIZE-summary bar, the VERSION chip and the SYNC chip have no client-shell code yet. Items 3.2 to 3.4 re-port them onto the files below; the old implementations to port from are in this branch's tree.

| Surface | Where in the 0.9.3 tree | Function | Tests that pin it |
|---|---|---|---|
| NAVIGATE / RESIZE / PREFIX / COPY / ERROR mode bar | `render.rs:31` (labels at 88/102/114/131/179; colour at ~58-70: mauve for RESIZE, accent for the rest; hint `segments` at ~83-190) | `render_mode_bar` | `tests/agents_worktrees_notifications.rs:1156`; `tests/startup_overlays.rs:752` (text only, none check style) |
| Mode bar restore | `composition.rs:3`, callers at 128 and 324 | `restore_mode_bar` | none |
| PANES bar + VERSION chip (to re-port) | not in the 0.9.3 tree; old code: `src/ui/display_panes.rs:200` (label at 231, VERSION spans 280-290, red at :73) | `render_summary_bar`, `mode_chip_style` | old tests in this branch: `display_panes.rs:460, 490, 516, 624` (the one at 490 says the VERSION chip is styled like PANES; 624 asserts PANES and VERSION `bg` are red) |
| RESIZE size summary (to re-port) | old code: `display_panes.rs:36/56` | `render_resize_mode_bar`, `render_window_resize_summary` | old tests `display_panes.rs:416, 532, 570, 593, 607` |
| SYNC chip and yellow borders (to re-port) | old code: `src/ui/panes.rs:~565-580` (`SYNC_YELLOW`, `sync_outsider()`); sync logic `src/app/sync_panes.rs` exists in all trees; no `SYNC` string draws in the client shell yet (3a65d4bc mentions the chip, not located) | find the draw site first (task 3.2a) | none found |
| Todos / notes panel | `todo_panel.rs`: layout 403 (`todo_panel_layout`), render 579 (`render_todo_panel`), detail box 535 (`render_detail_box`), footer buttons 361 (`footer_buttons`, the open-pane button is `TodoPanelButton::Go`). No search or count row yet | as listed | `tests/todo_overlays.rs:134, 189, 396, 427, 723` |
| Todo editor | `todo_edit.rs:123` layout, `:227` `render_todo_edit` | as listed | `tests/todo_overlays.rs:638` |
| Navigator footer | `overlays.rs:1036-1038` hint strings; `render_navigator_overlay` at 710 | | `tests/copy.rs:1654` (no footer text assertion) |
| Help footer | `overlays.rs:1242-1247` (`esc back`/`esc close` at 1152-1154); `render_help_overlay` at 1123, `help_lines` at 1059 | | `tests/input.rs:788`, `tests/keybindings_settings.rs:837` |
| Release notes / announcement footer | `overlays.rs:405`, `:521` | `render_release_notes_overlay` (314), `render_product_announcement_overlay` (430) | none |
| Notification centre footer | `notification_center.rs:80` (`footer_width`), `:93` (`footer_buttons`) | | `tests/notification_center.rs:152, 351` |

- [ ] 3.2a find where the SYNC chip is drawn on the 0.9.3 base (grep the sync state in `render.rs` / `composition.rs`) and add it to the table

