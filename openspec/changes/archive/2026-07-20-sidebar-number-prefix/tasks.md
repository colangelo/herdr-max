# Tasks: sidebar-number-prefix

## 1. Config

- [x] 1.1 `src/config/model.rs`: `ui.workspace_number_prefix` + `ui.agent_number_prefix` (String, default ""); `[ui]` parse test
- [x] 1.2 `src/app/state.rs` fields + `test_new()` defaults; `src/app/mod.rs` startup + live-reload
- [x] 1.3 `src/main.rs` template + config-reference JSON

## 2. Rendering

- [x] 2.1 `src/ui/sidebar.rs`: build the `{prefix}{symbol}` label; `editorial_number_reserve` reserves the label's display width (+gap); `draw_editorial_number` right-aligns the label overlay preserving the band
- [x] 2.2 Buffer test: editorial workspace row shows `₽<n>` right-aligned; empty prefix unchanged

## 3. Validation

- [x] 3.1 Docs: `docs/next/CHANGELOG.md`
- [x] 3.2 `just check`
- [ ] 3.3 Dogfood: beta, upgrade, set `workspace_number_prefix = "₽"` / `agent_number_prefix = "₽⌥"`, reload, user verifies; update+close issue #21
