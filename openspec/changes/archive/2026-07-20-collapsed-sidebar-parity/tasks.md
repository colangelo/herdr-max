# Tasks: collapsed-sidebar-parity

## 1. Rendering

- [x] 1.1 `src/ui/sidebar.rs` `render_sidebar_collapsed`: label space + agent rows with `jump_symbol`, 1-cell field; honour `workspace_number_color` / `agent_number_color` on non-active rows
- [x] 1.2 Active agent row: `is_active_pane` branch with `sidebar_active_band_bg()` band + bold `text` symbol; bold the active space symbol too
- [x] 1.3 `left`/`right` `sidebar_active_border`: reserve the edge column in collapsed rendering, draw `draw_sidebar_active_border_bar` on the active space row and active agent row
- [x] 1.4 `src/ui.rs`: collapsed sidebar width +1 when `sidebar_active_border` is `left`/`right`

## 2. Tests

- [x] 2.1 Update `collapsed_sidebar_keeps_status_visible_for_two_digit_positions`: 10th row shows `a` + gap + icon
- [x] 2.2 New buffer tests: active agent band+bold; number-color override applies collapsed; left border bar shifts content and draws on both active rows; off mode keeps width

## 3. Validation

- [x] 3.1 Docs: `docs/next/CHANGELOG.md`
- [x] 3.2 `cargo fmt` + `just check`
- [x] 3.3 Dogfood: beta, upgrade, user verifies collapsed sidebar live (16 agents, `sidebar_active_border = "left"`, custom `sidebar_active_bg`, number colors); then resolve+close issue #24 and archive
