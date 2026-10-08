Ordered so each group lands green on its own. Group 1 gives the existing entry
path a direction, group 2 binds the gestures to it.

## 1. A direction on the scroll-entry path

- [x] 1.1 A direction (up | down) threaded through
      `enter_copy_mode_scrolled` (`src/app/input/copy_mode.rs`) and
      `try_enter_app_scroll_mode` (`src/app/input/app_scroll.rs`), replacing the
      hardcoded upward scroll and the hardcoded `PageUp` / wheel-up entry sends
- [x] 1.2 Downward on a primary-screen pane with no active copy mode does
      nothing: no copy-mode entry, no mode change
- [x] 1.3 Tests: down on an alt-screen pane enters the passthrough and sends
      `PageDown`; the line variant sends a wheel-down tick; down on an
      unscrolled primary pane is a no-op; down scrolls an active copy mode and
      leaves its entry anchor alone; the upward gestures are unchanged

## 2. The bindings

- [x] 2.1 `KeysConfig.copy_mode_page_down` / `copy_mode_half_page_down` /
      `copy_mode_line_down` defaulting to `prefix+pagedown` / `prefix+ctrl+d` /
      `prefix+ctrl+j` (`src/config/model.rs`, `keybinds.rs`), the matching
      `NavigateAction` variants and both dispatchers
      (`src/app/input/navigate.rs`), `help_entry`s in
      `src/ui/keybind_help.rs`, commented entries in the `src/main.rs` template,
      and the `config-reference.json` rows
- [x] 2.2 The new actions join the set of prefix actions that do not cancel
      active copy mode, like their upward counterparts
- [x] 2.3 Tests: each default chord resolves to its action; the actions do not
      cancel copy mode; the help panel lists them
- [x] 2.4 Docs: keyboard page and the unreleased changelog
- [x] 2.5 `just check` green; dogfood on `-ac-beta` against the reported flow —
      scroll up in an alt-screen agent pane, switch space, come back, press
      `prefix+ctrl+d` and go down
