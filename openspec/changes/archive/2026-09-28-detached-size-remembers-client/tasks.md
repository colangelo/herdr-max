Planned 2026-09-28; built 2026-09-28 in the batch with dialog-overhaul, which ac approved.
Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/102

## 1. Tests first (red)

- [x] 1.1 headless: `detach_keeps_the_last_client_size` (310×56 attach → detach → effective size 310×56, no resize to 120×40)
- [x] 1.2 headless: `last_client_resize_wins` (200×50 → resize 310×56 → detach → 310×56)
- [x] 1.3 headless: `tiny_client_is_not_remembered` (310×56, then 60×20 attach/detach → 310×56)
- [x] 1.4 headless: `background_client_size_is_ignored` (foreground 310×56, background 100×30)
- [x] 1.5 headless: `handoff_size_beats_remembered_size` (handoff 200×60, remembered 310×56 → 200×60 until attach)
- [x] 1.6 headless: `remember_client_size_false_keeps_todays_behaviour`
- [x] 1.7 persist: `snapshot_round_trips_last_client_size`, `snapshot_without_last_client_size_loads`, `zero_last_client_size_is_ignored`
- [x] 1.8 startup: `cold_start_spawns_restored_panes_at_remembered_size`
- [x] 1.9 creation: split while detached still divides the target's real size (`a_split_while_detached_divides_the_target_at_a_remembered_size`; no end-to-end split test existed to re-run)

Found while building: a plain detach never resized existing PTYs (they keep
the last attached size; measured on beta.103, 173x49 before and after). The
headless size reached panes created while detached and every pane restored on
a cold start (93x39 = 120x40 minus the sidebar). Those are what this fixes.
Upstream's `pane_created_after_detach_uses_configured_headless_size` now runs
with `remember_client_size = false`; `pane_created_after_detach_uses_the_last_client_size`
covers the new default.

## 2. Build (green)

- [x] 2.1 `last_client_size` on the server + AppState; set from the foreground client with the 80×24 floor; mark the session dirty on change
- [x] 2.2 `detached_size()` precedence: handoff, remembered, headless
- [x] 2.3 `SessionSnapshot.last_client_size` (serde default); save and restore; seed before the first `detached_pane_size`
- [x] 2.4 `server.remember_client_size` (default true), wired at startup and in live reload (`apply_live_config`); `--default-config` template entry
- [x] 2.5 docs/next configuration.mdx (`[server]`), no CHANGELOG edit

## 3. Ship (with the batch)

- [x] 3.1 `just check` green; `openspec validate detached-size-remembers-client --strict`
- [x] 3.2 Beta; live proof on m4m: attach at full size, detach, `herdr-beta pane list` shows real widths; restart the server detached, panes come back at the remembered size; the nightly reads an unclipped footer
- [ ] 3.3 Comment the SHAs, beta and implementation notes on the issue; close it after the live check

Live proof, 2026-09-28, beta 0.8.2-ac-beta.104-chiesa (master da2ea021), throwaway
session on m4m: a 310x56 client's pane was 283x55; after the detach it stayed 283x55;
a workspace created detached got 310x56 (the no-client size, was 120x40); session.json
held `"last_client_size":[310,56]`; after `server stop` and a detached restart the
restored panes were 283x55 (beta.103 gave 93x39). The nightly's footer read was not
run here: it needs ac's real Claude Code panes.
