## Why

Reaching scrollback today is a two-step gesture: `prefix+[` to enter copy mode,
then a scroll key (`PageUp`, `Ctrl-U`, `Ctrl-K`, ...) to actually move. tmux
solves this with `bind PPage copy-mode -u` — one gesture that enters copy mode
*and* scrolls up — and that muscle memory is common. Herdr has no prefix-level
scroll bindings at all, so the most frequent scrollback intent ("look at what
just scrolled past") always costs an extra keystroke.

## What Changes

- Add three prefix-level actions that enter copy mode on the focused pane and
  perform the matching upward scroll in one gesture (tmux `copy-mode -u`
  semantics):
  - `copy_mode_page_up` — default `prefix+pageup`, full page
  - `copy_mode_half_page_up` — default `prefix+ctrl+u`, half page
  - `copy_mode_line_up` — default `prefix+ctrl+k`, one line
- Repeating a gesture while copy mode is already active on the focused pane
  scrolls further; it does not re-enter copy mode (which would clobber the
  entry scroll anchor and cursor). Exit semantics are unchanged: `q`/Esc still
  restores the pre-entry scroll position captured at first entry.
- Up-only by design: at the live bottom "down" has nowhere to go (tmux has no
  down variant either).
- `prefix+ctrl+b` is deliberately NOT a default: with the default `ctrl+b`
  prefix, `prefix+prefix` is send-prefix (literal prefix passthrough to the
  pane) and that check precedes binding dispatch, so the binding would be dead.
  Users with a non-`ctrl+b` prefix can bind it themselves; the config template
  comment says so.
- The three actions are discoverable in the `prefix+?` help panel and the
  generated config template.
- Non-goals: down-scroll entry variants, changing send-prefix precedence,
  honoring key repeat in prefix mode, or any new scroll machinery (the actions
  delegate to the existing copy-mode scroll primitives).

## Capabilities

### New Capabilities

- `copy-mode-scroll-entry`: One-gesture scrollback entry — prefix-level
  bindings that enter copy mode and scroll up by page, half page, or line,
  with repeat-gesture continuation and unchanged exit/restore semantics.

### Modified Capabilities

<!-- None. copy-mode-key-repeat and pane specs are untouched; this is additive
     prefix dispatch on top of existing copy-mode scroll primitives. -->

## Impact

- **Config** (`src/config/model.rs`): three new `KeysConfig` `BindingConfig`
  fields with defaults, profile/apply wiring, and parse-test rows.
- **Input dispatch** (`src/app/input/mod.rs`, `src/app/input/navigate.rs`):
  three new `NavigateAction` variants, binding lookup entries, membership in
  the copy-mode-survives set so re-invocation does not cancel copy mode.
- **Copy mode** (`src/app/input/copy_mode.rs`): a small entry helper that
  reuses `enter_copy_mode`, `scroll_copy_mode_page`, and
  `scroll_copy_mode_viewport_line`.
- **UI** (`src/ui/keybind_help.rs`): three help entries in the panes group.
- **Template** (`src/main.rs`): commented config entries.
- **Docs** (`docs/next/website/src/content/docs/keyboard.mdx`,
  `docs/next/CHANGELOG.md`): document the gesture.
- **Runtime/protocol**: none — TUI/client input projection only; headless
  parity is free because the headless path routes `Mode::Prefix` to the same
  `handle_prefix_key`. Boundary guardrail clean.
