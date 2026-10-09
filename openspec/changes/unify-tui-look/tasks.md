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

### Phase 3 file map (refreshed 2026-10-09 against `sync/merge` @ 240f6d5b, v0.9.3 + port)

Paths are `src/client/shell/` unless noted. The earlier map (7670867d) listed the PANES bar, VERSION chip and SYNC chip as having no client-shell code; the port has since re-landed them, so 3.2 and 3.3 restyle code that exists instead of re-porting it. Line numbers drift: find by function name.

| Surface | Where | Function / anchor | Tests that pin it |
|---|---|---|---|
| NAVIGATE / RESIZE / PREFIX / COPY / ERROR mode bar | `render.rs:34`; colours 58-72 (key style 59-61, `mode_style` 62-72: `palette.mauve` for Resize, `palette.accent` otherwise); labels ERROR 83, NAVIGATE 90, RESIZE 126, COPY 143 and 191 | `render_mode_bar` | `tests/keybindings_settings.rs:374` (pins `" NAVIGATE  esc back  "`), `tests/agents_worktrees_notifications.rs:1208` (contains only), `tests/startup_overlays.rs:752`, `tests/port_navigate.rs`, `tests/port_scroll.rs:239` (calls it directly); none check style except the last |
| Mode bar restore | `composition.rs:3`, callers 415, 586, 800 | `restore_mode_bar` | none |
| PANES bar, RESIZE summary, VERSION chip | `display_panes.rs`: `paint` 151, chip style 240-246 (`bg` mauve for resize, red otherwise), `summary_spans` 289, `" RESIZE "`/`" PANES "` 306, `" VERSION "` 346, pane borders red/`muted_red` 179 | `paint`, `summary_spans` | inline tests in `display_panes.rs` (~420), `tests/mod.rs:306` |
| SYNC chip | `sync_chrome.rs`: `paint` 4, text `" SYNC {n} pane(s) "` and `" SYNC ending… "` 20-24, `bg` `SYNC_YELLOW` 41. Constant: `src/app/state.rs:78`. Yellow pane borders still in `src/ui/panes.rs` (~673, ~1087, `sync_outsider()`); sync logic `src/app/sync_panes.rs` | `paint` | inline tests 51-93; `tests/port_sync.rs` |
| Todos / notes panel | `todo_panel.rs`: `footer_buttons` 367, `todo_panel_layout` 409, `render_detail_box` 541, `render_todo_panel` 585; the open-pane button is `TodoPanelButton::Go`, text `" g go "` (350). `todo_board.rs` exists beside it. No search or count row yet | as listed | `tests/todo_overlays.rs`, `tests/indicator_clicks.rs:141`, `tests/todo_board.rs` |
| Todo editor | `todo_edit.rs`: `todo_edit_layout` 133, `render_todo_edit` 237 (panel shell in `p.accent`) | as listed | `tests/todo_overlays.rs` |
| Navigator footer | `overlays.rs`: `render_navigator_overlay` 911; hint strings need re-finding (old 1036-1038 is stale) | | `tests/navigator_overlays.rs`, `tests/copy.rs` |
| Help footer | `overlays.rs`: `help_lines` 1331, `render_help_overlay` 1395, `esc back` / `esc close` button 1420-1422 | | `tests/input.rs`, `tests/keybindings_settings.rs` |
| Release notes / announcement footer | `overlays.rs`: `render_release_notes_overlay` 429 (footer `" esc close "` 493), `render_product_announcement_overlay` 545 (609) | | `tests/startup_overlays.rs` |
| Notification centre footer | `notification_center.rs`: `footer_width` 80, `footer_buttons` 93 | | `tests/notification_center.rs` |

Still absent from the client shell (old files removed by 8a5d2b91): `src/ui/navigator.rs`, `menus.rs`, `todo_panel.rs`, `display_panes.rs`; their client-shell successors are the files above.

- [x] 3.2a find where the SYNC chip is drawn on the 0.9.3 base: `sync_chrome.rs` `paint`
