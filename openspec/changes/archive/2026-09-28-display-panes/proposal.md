## Why

When copying text between panes, or when a menu or program lays itself out to
the terminal, it helps to know each pane's address and its size in characters.
Today that means running `herdr pane layout` and reading JSON. tmux answers the
same question with one key (`prefix q`, display-panes): every pane shows its
number and size until the next key. Asked for by ac on 2026-09-28 (JOB 7 via
herdr-relay); ac added on the same day that the label must carry the pane's
name as well as its size, and the full window size must show somewhere.

## What Changes

- A new keybinding action `keys.display_panes`, default `prefix+i`
  ("identify"). `prefix+q` is tmux's key but is herdr's default `detach`.
- While it is shown, every pane in the current tab carries a label over its
  centre: an index `1`–`9`, its public address, its name and its terminal
  size in characters, e.g. `2  w5:p16 · claude  138x27`. The mode bar shows the whole window size and the pane
  area, e.g. `window 310x56 · panes 281x55`.
- It closes after 3 seconds or on any key. A digit that names a shown pane
  focuses that pane first. A mouse press also closes it.
- A `help_entry` makes it discoverable in `prefix ?`.

## Impact

- TUI presentation only: an overlay in the `overlays!` list, rendered from the
  view geometry the renderer already has. No new server-side fact, API field,
  event or protocol change. Focusing by number goes through the existing
  `pane.focus` runtime path.
- `src/app/state.rs`, `src/app/input/*`, `src/app/runtime.rs`,
  `src/server/headless.rs` (timeout), `src/ui/*`, `src/config/*`, `src/main.rs`
  (config template), `docs/next/website/…/keyboard.mdx` and the config
  reference data.
