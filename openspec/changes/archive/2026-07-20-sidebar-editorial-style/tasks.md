# Tasks: sidebar-editorial-style

## 1. Config surface

- [x] 1.1 `src/config/model.rs`: `SidebarStyleConfig` enum (`default`/`editorial`) + `ui.sidebar_style` field; `StateColorsConfig` table (`working`/`idle`/`done`/`blocked`/`unknown`, all `Option<String>`) + `ui.state_colors`; defaults; `[ui]` parse tests; re-exports in `src/config.rs`
- [x] 1.2 `src/app/state.rs`: `sidebar_style` + parsed state-color fields with `test_new()` defaults; `AppState::state_icon_colors()` resolver with palette fallback; `src/app/mod.rs`: startup + live-reload wiring
- [x] 1.3 `src/main.rs` config template entries; config-reference JSON entries

## 2. State colors

- [x] 2.1 `src/ui/status.rs`: `state_dot` / `agent_icon` / `state_label_color` take the resolved state-color struct; update all call sites (expanded + collapsed, spaces + agents); default-mode output identical (unit test glyph+color mapping with and without overrides)

## 3. Editorial rendering

- [x] 3.1 `render_workspace_list`: in editorial mode skip the row-1 number prefix, reserve `symbol_width + 1` from the name row's token budget, and overlay the right-aligned number paragraph (transparent bg over the pre-filled band)
- [x] 3.2 `render_agent_detail`: same, with the overlay carrying `row_style` so the active band has no gap
- [x] 3.3 Headers: uppercase, no bold, dimmed in editorial mode (spaces + agents); sort-toggle label untouched
- [x] 3.4 Meta line: DIM on inactive secondary style in editorial mode; active entry keeps accent branch color
- [x] 3.5 Buffer tests (TestBackend): editorial name row shows right-aligned number with active band intact; default mode renders unchanged (characterization)

## 4. Validation

- [x] 4.1 Docs: changelog entry in `docs/next/CHANGELOG.md`
- [x] 4.2 `just check` passes
- [ ] 4.3 Live dogfood: beta build, upgrade, set `ui.sidebar_style = "editorial"`, `[ui.state_colors] working = "#ffc832"`, `idle = "#4ade80"`, `done = "#4ade80"`, number colors `#8f4747`, `row_gap = 1`; user verifies against the approved mockup
- [ ] 4.4 Update and close Gitea issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/20 after verification
